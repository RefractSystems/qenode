import asyncio
import logging
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import pytest
import pytest_asyncio
import zenoh

from tools.testing.qmp_bridge import QmpBridge

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


def pack_clock_advance(delta_ns: int, mujoco_time_ns: int = 0) -> bytes:
    return struct.pack("<QQ", delta_ns, mujoco_time_ns)


def unpack_clock_ready(data: bytes) -> tuple[int, int, int]:
    return struct.unpack("<QII", data)


async def wait_for_zenoh_discovery(session: zenoh.Session, topic: str, expected_count: int = 1, timeout: float = 30.0):
    """
    Blocks until Zenoh discovery confirms the network mesh is established.
    Empirically verifies router connectivity.
    """
    logger.info(f"Zenoh: verifying connectivity for {topic} (expected={expected_count})...")
    start = asyncio.get_running_loop().time()

    while asyncio.get_running_loop().time() - start < timeout:
        # Check if we see at least one router or peer.
        # In our test topology, there is usually 1 persistent router.
        try:
            routers = list(session.info.routers_zid())
            if len(routers) >= 1:
                logger.info(f"Zenoh: mesh established (routers={len(routers)})")

                # Give Zenoh's discovery protocol time to exchange routing tables
                # and establish pub/sub matching across the mesh.
                grace_period = 3.0 if os.environ.get("VIRTMCU_USE_ASAN") == "1" else 0.5
                await asyncio.sleep(grace_period)
                return
        except Exception:
            pass

        await asyncio.sleep(0.1)

    raise TimeoutError(f"Zenoh discovery timeout for {topic} after {timeout}s")


# VTA step timeout: always longer than the QEMU stall-timeout so QEMU can reply
# with STALL before Python gives up. VIRTMCU_STALL_TIMEOUT_MS drives both sides:
# QEMU reads it directly; Python adds a 10-second buffer on top.
if os.environ.get("VIRTMCU_USE_ASAN") == "1" and "VIRTMCU_STALL_TIMEOUT_MS" not in os.environ:
    os.environ["VIRTMCU_STALL_TIMEOUT_MS"] = "300000"

_stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
_DEFAULT_VTA_STEP_TIMEOUT_S: float = max(60.0, _stall_timeout_ms / 1000.0 + 10.0)


def pytest_configure(config: pytest.Config) -> None:
    """Scale pytest-timeout's global threshold when VIRTMCU_STALL_TIMEOUT_MS is elevated.

    The pyproject.toml default (120 s) suits non-ASan runs. Under ASan/UBSan
    QEMU executes TCG blocks much more slowly; a 100 ms virtual-time step can
    legitimately take several minutes of wall-clock time. We raise the per-test
    limit to _DEFAULT_VTA_STEP_TIMEOUT_S + 60 s so pytest-timeout never kills a
    test that is still making forward progress toward the QEMU stall boundary.

    pytest-timeout caches the resolved timeout in config._env_timeout during its
    own pytest_configure (which runs before conftest hooks).  Setting
    config.option.timeout afterwards has no effect — we must update _env_timeout
    directly.
    """
    computed = _DEFAULT_VTA_STEP_TIMEOUT_S + 60.0
    # pytest-timeout caches its resolved timeout in config._env_timeout during its
    # own pytest_configure (which runs before conftest hooks).  Modifying
    # config.option.timeout afterwards has no effect on the cached value.  We update
    # _env_timeout through __dict__ to avoid both the ruff B010 (setattr constant)
    # and mypy attr-defined errors while remaining lint-clean.
    config_dict = vars(config)
    current_timeout = float(config_dict.get("_env_timeout") or 0.0)
    if computed > current_timeout:
        config_dict["_env_timeout"] = computed


class VirtualTimeAuthority:
    """
    Enterprise-grade controller for driving multiple QEMU virtual clocks via Zenoh.
    """

    def __init__(self, session: zenoh.Session, node_ids: list[int]):
        self.session = session
        self.node_ids = node_ids
        self.current_vtimes = dict.fromkeys(node_ids, 0)

    async def step(self, delta_ns: int, timeout: float = _DEFAULT_VTA_STEP_TIMEOUT_S) -> Any:
        """
        Advances the clock of all managed nodes.
        Timeout scales with VIRTMCU_STALL_TIMEOUT_MS so ASan builds get enough headroom.
        """
        tasks = []
        for nid in self.node_ids:
            topic = f"sim/clock/advance/{nid}"
            payload = pack_clock_advance(delta_ns)
            tasks.append(self._get_reply(nid, topic, payload, timeout))

        replies = await asyncio.gather(*tasks)

        for nid, reply in zip(self.node_ids, replies, strict=True):
            if not reply:
                raise TimeoutError(f"Node {nid} failed to respond to clock advance within {timeout}s")
            if not reply.ok:
                raise RuntimeError(f"Node {nid} returned Zenoh error: {reply.err}")

            vtime, _mujoco_vtime, error_code = unpack_clock_ready(reply.ok.payload.to_bytes())
            if error_code != 0:
                # 1 = STALL
                raise RuntimeError(
                    f"Node {nid} reported CLOCK STALL (error={error_code}) at vtime={vtime}. "
                    f"QEMU failed to reach TB boundary within its stall-timeout."
                )

            self.current_vtimes[nid] = vtime

        return self.current_vtimes

    async def run_for(self, duration_ns: int, step_ns: int = 10_000_000) -> int:
        """
        Advances all clocks by duration_ns.
        """
        target = min(self.current_vtimes.values()) + duration_ns
        while min(self.current_vtimes.values()) < target:
            to_step = min(step_ns, target - min(self.current_vtimes.values()))
            await self.step(to_step)
        return min(self.current_vtimes.values())

    async def _get_reply(self, nid, topic, payload, timeout):
        def _sync_get():
            try:
                for r in self.session.get(topic, payload=payload, timeout=timeout):
                    return r
            except Exception as e:
                logger.warning(f"[VTA] Node {nid} GET error on {topic}: {e}")
            return None

        return await asyncio.to_thread(_sync_get)


@pytest_asyncio.fixture
async def zenoh_router():
    """
    Fixture that starts a persistent Zenoh router for the duration of the test.
    Supports pytest-xdist parallelization by dynamically binding to a free port.
    """
    # worker_id is provided by pytest-xdist. Fallback to 'master' if not running in parallel.
    # worker_id = getattr(request.config, "workerinput", {}).get("workerid", "master")

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

    # Wait for router to bind the socket
    import socket

    host = "127.0.0.1"
    for _ in range(50):
        try:
            with socket.create_connection((host, port), timeout=0.1):
                break
        except (TimeoutError, ConnectionRefusedError):
            await asyncio.sleep(0.1)
    else:
        raise RuntimeError(f"Zenoh Router failed to bind to {endpoint}")

    # Wait for router to be ready internally
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{endpoint}"]')
    config.insert_json5("mode", '"client"')

    check_session = await asyncio.to_thread(lambda: zenoh.open(config))
    try:
        await wait_for_zenoh_discovery(check_session, "sim/router/check")
    finally:
        await asyncio.to_thread(check_session.close)

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
    return VirtualTimeAuthority(zenoh_session, [0])


@pytest_asyncio.fixture
async def zenoh_coordinator(zenoh_router):
    """
    Fixture that starts the zenoh_coordinator.
    """
    curr = Path(Path(__file__).resolve().parent)
    while str(curr) != "/" and not (curr / "tools").exists():
        curr = Path(curr).parent
    workspace_root = curr

    # First check if CARGO_TARGET_DIR is set (used by CI)
    import os

    if "CARGO_TARGET_DIR" in os.environ:
        coord_bin = Path(os.environ["CARGO_TARGET_DIR"]) / "release/zenoh_coordinator"
    else:
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
        if "CARGO_TARGET_DIR" in os.environ:
            coord_bin = Path(os.environ["CARGO_TARGET_DIR"]) / "release/zenoh_coordinator"
        else:
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
        bridge.pid = proc.pid
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
            await asyncio.wait_for(inst["bridge"].close(), timeout=5.0)
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


class TimeAuthority(VirtualTimeAuthority):
    """
    Legacy wrapper for VirtualTimeAuthority that drives a single node.
    """

    def __init__(self, session: zenoh.Session, node_id: int):
        super().__init__(session, [node_id])

    @property
    def current_vtime_ns(self) -> int:
        return self.current_vtimes[self.node_ids[0]]

    async def step(self, delta_ns: int, timeout: float = 60.0) -> Any:
        res = await super().step(delta_ns, timeout)
        return res[self.node_ids[0]]
