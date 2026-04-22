import asyncio
import logging
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import pytest_asyncio
import zenoh

from tools.testing.qmp_bridge import QmpBridge
from tools.vproto import ClockAdvanceReq, ClockReadyResp

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


class TimeAuthority:
    """
    Helper to drive QEMU virtual clock via Zenoh.
    """

    def __init__(self, session: zenoh.Session, node_id: int = 0):
        self.session = session
        self.topic = f"sim/clock/advance/{node_id}"
        self.current_vtime_ns = 0

    async def step(self, delta_ns: int, timeout: float = 60.0, delay: float = 0, retries: int = 50) -> int:
        target_vtime = self.current_vtime_ns + delta_ns
        req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0)
        logger.info(f"TimeAuthority: stepping {delta_ns}ns (target={target_vtime}) on {self.topic}")

        if delay > 0:
            await asyncio.sleep(delay)

        # We retry because discovery can take time in parallel CI runs.
        # Use an async iterator to avoid blocking for the full timeout.
        reply = None

        def _get_first_reply():
            # In Zenoh 1.0, get() returns an iterable.
            # We want to break as soon as we get ONE valid reply.
            try:
                for r in self.session.get(self.topic, payload=req.pack(), timeout=timeout):
                    return r
            except Exception as e:
                logger.warning(f"TimeAuthority: Zenoh get error: {e}")
            return None

        for i in range(retries):
            reply = await asyncio.to_thread(_get_first_reply)
            if reply:
                break
            if i < retries - 1:
                wait_time = min(2.0, 0.5 * (i + 1))  # Cap wait at 2 seconds
                logger.warning(
                    f"TimeAuthority: no reply from {self.topic}, retrying in {wait_time}s... ({i + 1}/{retries})"
                )
                await asyncio.sleep(wait_time)

        if not reply:
            logger.error(f"TimeAuthority: NO REPLIES from {self.topic} after {retries} attempts")
            raise TimeoutError(f"TimeAuthority: no reply from {self.topic}")

        if reply.ok:
            resp = ClockReadyResp.unpack(reply.ok.payload.to_bytes())
            logger.info(
                f"TimeAuthority: received reply: current_vtime={resp.current_vtime_ns}, error={resp.error_code}"
            )
            if resp.error_code != 0:
                logger.warning(f"TimeAuthority: error_code={resp.error_code}")
                # For stall tests, return the error code if it's not OK
                return resp.error_code

            # Update current vtime with actual time reached by QEMU
            self.current_vtime_ns = resp.current_vtime_ns
            return int(self.current_vtime_ns)

        logger.error(f"TimeAuthority: ERROR REPLY from {self.topic}")
        raise RuntimeError(f"TimeAuthority: error reply from {self.topic}")

    async def step_vtime(self, delta_ns: int, timeout: float = 60.0, delay: float = 0) -> int:
        """Same as step but returns the vtime returned by QEMU."""
        return await self.step(delta_ns, timeout, delay)


@pytest_asyncio.fixture
async def zenoh_router(worker_id):  # noqa: ARG001
    """
    Fixture that starts a persistent Zenoh router for the duration of the test.
    Supports pytest-xdist parallelization by dynamically binding to a free port.
    """

    tests_dir = Path(Path(__file__).resolve().parent)
    router_script = Path(tests_dir) / "zenoh_router_persistent.py"
    workspace_root = tests_dir.parent
    get_port_script = workspace_root / "scripts/get-free-port.py"

    # Find a dynamically free port using our utility
    proc_port = await asyncio.create_subprocess_exec(
        sys.executable,
        str(get_port_script),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, _ = await proc_port.communicate()
    port = int(stdout.decode().strip())

    endpoint = f"tcp/127.0.0.1:{port}"

    logger.info(f"Starting Zenoh Router on {endpoint}...")

    # We MUST NOT run global cleanup like 'make clean-sim' here as it would kill other parallel tests!

    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        "-u",
        str(router_script),
        endpoint,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )

    async def _stream_router_output(stream, name):
        while True:
            line = await stream.readline()
            if not line:
                break
            logger.info(f"Zenoh Router {name}: {line.decode().strip()}")

    _router_tasks = [
        asyncio.create_task(_stream_router_output(proc.stdout, "STDOUT")),
        asyncio.create_task(_stream_router_output(proc.stderr, "STDERR")),
    ]

    # Wait for router to be ready
    await asyncio.sleep(1.0)

    yield endpoint

    if proc.returncode is None:
        proc.terminate()
        try:
            await asyncio.wait_for(proc.wait(), timeout=2.0)
        except TimeoutError:
            proc.kill()
            await proc.wait()


@pytest_asyncio.fixture
async def zenoh_session(zenoh_router):
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{zenoh_router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    config.insert_json5("mode", '"client"')
    # Task 27.3: Increase task workers to prevent deadlocks when blocking in query handlers.
    import contextlib

    with contextlib.suppress(Exception):
        config.insert_json5("transport/shared/task_workers", "16")
    session = await asyncio.to_thread(lambda: zenoh.open(config))

    # Wait for session to connect to the router (either as a router or a peer)
    connected = False
    for _ in range(100):
        info = session.info
        if list(info.routers_zid()) or list(info.peers_zid()):
            connected = True
            break
        await asyncio.sleep(0.1)

    if not connected:
        await asyncio.to_thread(session.close)
        raise RuntimeError(f"Failed to connect Zenoh session to {zenoh_router}")

    yield session
    await asyncio.to_thread(session.close)


@pytest_asyncio.fixture
async def time_authority(zenoh_session):
    return TimeAuthority(zenoh_session)


@pytest_asyncio.fixture
async def zenoh_coordinator(zenoh_router):
    """
    Fixture that starts the zenoh_coordinator.
    """
    curr = Path(Path(__file__).resolve().parent)
    while str(curr) != "/" and not (curr / "tools").exists():
        curr = Path(curr).parent
    workspace_root = curr

    # Try standard Cargo target directory at workspace root
    coord_bin = workspace_root / "target/release/zenoh_coordinator"

    # Also check the tool-local target dir
    if not coord_bin.exists():
        coord_bin = workspace_root / "tools/zenoh_coordinator/target/release/zenoh_coordinator"

    # Use a lock to build once in parallel runs
    if not coord_bin.exists():
        lock_file = workspace_root / "tools/zenoh_coordinator/build.lock"
        import fcntl

        with lock_file.open("w") as f:
            try:
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                if not coord_bin.exists():
                    logger.info("Building zenoh_coordinator...")
                    proc = await asyncio.create_subprocess_exec(
                        "cargo", "build", "--release", cwd=(workspace_root / "tools/zenoh_coordinator")
                    )
                    await proc.wait()
            except BlockingIOError:
                logger.info("Waiting for zenoh_coordinator build...")
                for _ in range(60):
                    if coord_bin.exists():
                        break
                    await asyncio.sleep(1.0)

        # Refresh location after build
        coord_bin = workspace_root / "target/release/zenoh_coordinator"
        if not coord_bin.exists():
            coord_bin = workspace_root / "tools/zenoh_coordinator/target/release/zenoh_coordinator"

    logger.info(f"Starting Zenoh Coordinator connecting to {zenoh_router}...")

    cmd = [str(coord_bin), "--connect", zenoh_router]

    proc = await asyncio.create_subprocess_exec(
        *cmd,
        stdout=None,
        stderr=None,
        env=os.environ.copy(),
    )

    await asyncio.sleep(1.0)

    yield proc

    if proc.returncode is None:
        proc.terminate()
        try:
            await asyncio.wait_for(proc.wait(), timeout=2.0)
        except TimeoutError:
            proc.kill()
            await proc.wait()


@pytest_asyncio.fixture
async def qemu_launcher():
    """
    Fixture that returns a function to launch QEMU instances.
    Ensures all instances are cleaned up after the test.
    """
    instances: list[dict[str, Any]] = []

    async def _launch(dtb_path, kernel_path=None, extra_args=None, ignore_clock_check=False):
        # Create a unique temporary directory for this QEMU instance
        tmpdir = tempfile.mkdtemp(prefix="virtmcu-test-")
        qmp_sock = Path(tmpdir) / "qmp.sock"
        uart_sock = Path(tmpdir) / "uart.sock"

        # Build the command using run.sh
        curr = Path(Path(__file__).resolve().parent)
        while str(curr) != "/" and not (curr / "scripts").exists():
            curr = Path(curr).parent
        workspace_root = curr
        run_script = Path(workspace_root) / "scripts/run.sh"

        cmd: list[str] = [str(run_script), "--dtb", str(Path(dtb_path).resolve())]
        if kernel_path:
            cmd.extend(["--kernel", str(Path(kernel_path).resolve())])

        # Add QMP and UART sockets
        cmd.extend(
            [
                "-qmp",
                f"unix:{qmp_sock},server,nowait",
                "-display",
                "none",
                "-nographic",
            ]
        )

        # Only add default serial if not overridden in extra_args
        has_serial = False
        if extra_args:
            for arg in extra_args:
                if arg in ["-serial", "-chardev"]:
                    has_serial = True
                    break

        if not has_serial:
            cmd.extend(["-serial", f"unix:{uart_sock},server,nowait"])

        if extra_args:
            cmd.extend(extra_args)

        # Task 4.1b: Critical isolation constraint - standalone mode only
        if not ignore_clock_check:
            for arg in cmd:
                if "zenoh-clock" in str(arg):
                    raise ValueError(
                        "zenoh-clock device detected in standalone test suite. "
                        "Phase 4 tests must run without external clock plugins."
                    )

        logger.info(f"Launching QEMU: {' '.join(cmd)}")

        # Start the process
        proc = await asyncio.create_subprocess_exec(
            *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE, env=os.environ.copy()
        )

        async def _stream_output(stream, name):
            while True:
                line = await stream.readline()
                if not line:
                    break
                logger.info(f"QEMU {name}: {line.decode().strip()}")

        # Task 4.2d: Stream QEMU output in background for better debuggability.
        # We store task references to prevent them from being garbage collected.
        output_tasks = [
            asyncio.create_task(_stream_output(proc.stdout, "STDOUT")),
            asyncio.create_task(_stream_output(proc.stderr, "STDERR")),
        ]

        # Wait for sockets to be created by QEMU.
        retries = 100
        while retries > 0:
            if proc.returncode is not None:
                stdout, stderr = await proc.communicate()
                raise RuntimeError(
                    f"QEMU exited unexpectedly (rc={proc.returncode}) before sockets appeared.\n"
                    f"STDOUT: {stdout.decode()}\nSTDERR: {stderr.decode()}"
                )
            if Path(qmp_sock).exists() and (has_serial or Path(uart_sock).exists()):
                break
            await asyncio.sleep(0.1)
            retries -= 1
        else:
            proc.terminate()
            stdout, stderr = await proc.communicate()
            logger.error(f"QEMU failed to start. STDOUT: {stdout.decode()} STDERR: {stderr.decode()}")
            raise TimeoutError("QEMU QMP/UART sockets did not appear in time")

        bridge = QmpBridge()
        try:
            await bridge.connect(str(qmp_sock), None if has_serial else str(uart_sock))
        except Exception as e:
            stdout, stderr = await proc.communicate()
            logger.error(
                f"QEMU failed to establish connection. rc={proc.returncode}\n"
                f"STDOUT: {stdout.decode()}\nSTDERR: {stderr.decode()}"
            )
            raise e

        instance = {"proc": proc, "bridge": bridge, "tmpdir": tmpdir, "cmd": cmd, "output_tasks": output_tasks}
        instances.append(instance)
        return bridge

    yield _launch

    # Cleanup
    for inst in instances:
        try:
            await inst["bridge"].close()
        except Exception as e:
            logger.warning(f"Error closing bridge: {e}")

        proc = inst["proc"]
        if proc.returncode is None:
            proc.terminate()
            try:
                await asyncio.wait_for(proc.wait(), timeout=5.0)
            except TimeoutError:
                proc.kill()
                await proc.wait()

        # Always capture and print output if the test failed
        stdout, stderr = await proc.communicate()
        if stdout or stderr:
            print(f"\n--- QEMU Output for {' '.join(inst['cmd'])} ---")
            if stdout:
                print(f"STDOUT:\n{stdout.decode()}")
            if stderr:
                print(f"STDERR:\n{stderr.decode()}")
            print("------------------------------------------")

        shutil.rmtree(inst["tmpdir"], ignore_errors=True)


@pytest_asyncio.fixture
async def qmp_bridge(qemu_launcher):
    dtb = "test/phase1/minimal.dtb"
    kernel = "test/phase1/hello.elf"
    if not Path(dtb).exists():
        subprocess.run(["make", "-C", "test/phase1", "minimal.dtb"], check=True)
    bridge = await qemu_launcher(dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()
    return bridge
