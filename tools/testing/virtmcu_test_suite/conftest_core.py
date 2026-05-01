import asyncio
import contextlib
import logging
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import pytest
import pytest_asyncio
import vproto
import zenoh

from tools.testing.qmp_bridge import QmpBridge
from tools.testing.utils import get_time_multiplier, wait_for_file_creation, yield_now

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


def pack_clock_advance(delta_ns: int, mujoco_time_ns: int = 0, quantum_number: int = 0) -> bytes:
    return vproto.ClockAdvanceReq(delta_ns, mujoco_time_ns, quantum_number).pack()


def unpack_clock_ready(data: bytes) -> tuple[int, int, int, int]:
    resp = vproto.ClockReadyResp.unpack(data)
    return resp.current_vtime_ns, resp.n_frames, resp.error_code, resp.quantum_number


def get_zenoh_router_endpoint(session: zenoh.Session) -> str:
    """
    Returns the first connected endpoint from a Zenoh session.
    Used by tests to dynamically find the router address.
    """
    # Prefer environment variable if set
    if "VIRTMCU_ZENOH_ROUTER" in os.environ:
        return os.environ["VIRTMCU_ZENOH_ROUTER"]

    # In some versions of zenoh-python info is a property, in others a method.
    try:
        info = session.info if not callable(session.info) else session.info()

        # SessionInfo might not be a dict in some versions
        if hasattr(info, "get"):
            endpoints = info.get("connect/endpoints", [])
        else:
            endpoints = getattr(info, "connect/endpoints", [])

        if endpoints and len(endpoints) > 0:
            return str(endpoints[0])
    except Exception as e:
        logger.debug(f"Note: Zenoh session info not available yet: {e}")

    raise RuntimeError(
        "Failed to discover Zenoh router endpoint from session and VIRTMCU_ZENOH_ROUTER is not set. "
        "Ensure the Zenoh router is started and the session is connected."
    )


async def wait_for_zenoh_discovery(
    session: zenoh.Session, topic: str, expected_count: int = 1, timeout: float | None = 15.0
):
    """
    Blocks until Zenoh discovery confirms the network mesh is established.
    Uses the Zenoh Liveliness API for deterministic signaling without polling or sleeps.
    """
    real_timeout: float = (timeout if timeout is not None else 15.0) * get_time_multiplier()

    logger.info(f"Zenoh: waiting for liveliness on {topic} (expected={expected_count})...")

    # We use both Liveliness AND a periodic check to handle edge cases
    event = asyncio.Event()
    loop = asyncio.get_running_loop()

    def on_liveliness(sample):
        if sample.kind == zenoh.SampleKind.PUT:
            logger.info(f"Zenoh: liveliness detected on {topic} (PUT)")
            loop.call_soon_threadsafe(event.set)

    # 1. Subscribe to liveliness changes
    sub = await asyncio.to_thread(lambda: session.liveliness().declare_subscriber(topic, on_liveliness))

    try:
        # 2. Polling check for existing liveliness (backup for race condition)
        # We do this in a loop until timeout or success
        start_time = loop.time()
        while True:

            def check_current():
                replies = session.liveliness().get(topic)
                for _ in replies:
                    return True
                return False

            if await asyncio.to_thread(check_current):
                logger.info(f"Zenoh: {topic} is already alive (polled)")
                # VERIFY: In addition to liveliness, check if we can actually perform a GET
                # to the corresponding advance topic if applicable.
                if "/liveliness/" in topic:
                    nid = topic.split("/")[-1]
                    advance_topic = f"sim/clock/advance/{nid}"

                    # Try a very short-timeout GET with dummy payload
                    def probe_get(t=advance_topic):
                        try:
                            # Use a GET with a timeout to see if anybody is listening
                            _ = session.get(t, zenoh.Queue(), timeout=0.5)  # type: ignore
                            # We don't care if it times out, we just want to see if it doesn't immediately fail
                            # Actually, if there are NO queryables, it returns immediately empty.
                            # If there ARE queryables but they are busy, it might timeout.
                            return True  # For now, just being alive is enough, but logged.
                        except Exception:
                            return False

                    await asyncio.to_thread(probe_get)

                return

            elapsed = loop.time() - start_time

            if elapsed >= real_timeout:
                raise TimeoutError(f"Zenoh discovery timeout for {topic} after {real_timeout}s")

            try:
                # Wait for the subscriber event or a small polling interval
                await asyncio.wait_for(event.wait(), timeout=min(1.0, real_timeout - elapsed))
                logger.info(f"Zenoh: {topic} confirmed via subscriber event")
                return
            except TimeoutError:
                continue
    finally:
        await asyncio.to_thread(sub.undeclare)


# VTA step timeout: always longer than the QEMU stall-timeout so QEMU can reply
# with STALL before Python gives up. VIRTMCU_STALL_TIMEOUT_MS drives both sides:
# QEMU reads it directly; Python adds a 10-second buffer on top.
_base_stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
_stall_timeout_ms = int(_base_stall_timeout_ms * get_time_multiplier())
_DEFAULT_VTA_STEP_TIMEOUT_S: float = max(60.0, _stall_timeout_ms / 1000.0 + 10.0)


class VirtualTimeAuthority:
    """
    Enterprise-grade controller for driving multiple QEMU virtual clocks via Zenoh.
    """

    def __init__(self, session: zenoh.Session, node_ids: list[int]):
        self.session = session
        self.node_ids = node_ids
        self.pacing_multiplier = float(os.environ.get("VIRTMCU_PACING_MULTIPLIER", "0.0"))
        self.start_wall_time = None
        self.start_vtime_ns = None
        self.current_vtimes = dict.fromkeys(node_ids, 0)
        self._expected_vtime_ns = dict.fromkeys(node_ids, 0)
        self._overshoot_ns = dict.fromkeys(node_ids, 0)
        self.quantum_number = 0
        self._liveliness_checked = False

    async def init(self, timeout: float = 30.0):
        """
        Deterministic Initialization Barrier.
        """
        if not self._liveliness_checked:
            liveliness_tasks = []
            for nid in self.node_ids:
                hb_topic = f"sim/clock/liveliness/{nid}"
                liveliness_tasks.append(wait_for_zenoh_discovery(self.session, hb_topic, timeout=timeout))
            await asyncio.gather(*liveliness_tasks)
            self._liveliness_checked = True

        # SAFETY: Give QEMU a tiny bit of slack to finish its internal transition to the first barrier
        await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: initialization slack

        # Perform the 0-ns sync to ensure QEMU is perfectly frozen and ready
        await self.step(0, timeout=timeout)

    async def step(self, delta_ns: int, timeout: float = _DEFAULT_VTA_STEP_TIMEOUT_S) -> Any:
        if timeout is not None:
            timeout *= get_time_multiplier()

        """
        Advances the clock of all managed nodes.
        Timeout scales with VIRTMCU_STALL_TIMEOUT_MS so ASan builds get enough headroom.
        """
        tasks = []
        self.quantum_number += 1
        for nid in self.node_ids:
            topic = f"sim/clock/advance/{nid}"

            # Compensate for accumulated overshoot from previous quantum.
            adjusted_delta = max(0, delta_ns - self._overshoot_ns[nid])
            target_mujoco_time = self._expected_vtime_ns[nid] + delta_ns

            payload = pack_clock_advance(adjusted_delta, target_mujoco_time, self.quantum_number)
            tasks.append(self._get_reply(nid, topic, payload, timeout))

        replies = await asyncio.gather(*tasks)

        for nid, reply in zip(self.node_ids, replies, strict=True):
            if not reply:
                raise TimeoutError(f"Node {nid} failed to respond to clock advance within {timeout}s")
            if not reply.ok:
                raise RuntimeError(f"Node {nid} returned Zenoh error: {reply.err}")

            vtime, _n_frames, error_code, qn = unpack_clock_ready(reply.ok.payload.to_bytes())
            if error_code != 0:
                # 1 = STALL
                raise RuntimeError(
                    f"Node {nid} reported CLOCK STALL (error={error_code}) at vtime={vtime}. "
                    f"QEMU failed to reach TB boundary within its stall-timeout."
                )
            if qn != self.quantum_number:
                raise RuntimeError(
                    f"Node {nid} returned wrong quantum_number: expected {self.quantum_number}, got {qn}"
                )

            self.current_vtimes[nid] = vtime
            self._expected_vtime_ns[nid] += delta_ns
            self._overshoot_ns[nid] = max(0, vtime - self._expected_vtime_ns[nid])

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

    curr = Path(Path(__file__).resolve().parent)
    while str(curr) != "/" and not (curr / "scripts").exists():
        curr = Path(curr).parent
    workspace_root = curr
    router_script = workspace_root / "tests/zenoh_router_persistent.py"
    get_port_script = workspace_root / "scripts/get-free-port.py"

    # Find a dynamically free endpoint using our utility
    proc_port = await asyncio.create_subprocess_exec(
        sys.executable,
        str(get_port_script),
        "--endpoint",
        "--proto",
        "tcp/",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, _ = await proc_port.communicate()
    endpoint = stdout.decode().strip()

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

    # Wait for router to be ready internally
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{endpoint}"]')
    config.insert_json5("mode", '"client"')

    # zenoh.open() in client mode with connect/endpoints may throw ZError
    # if the TCP port is not yet open. We retry with a small yield.
    check_session = None
    for _ in range(int(100 * get_time_multiplier())):
        try:
            check_session = await asyncio.to_thread(lambda: zenoh.open(config))
            break
        except Exception:
            # TCP port opening is non-deterministic; backoff and retry.
            await asyncio.sleep(0.05)  # SLEEP_EXCEPTION: waiting for Zenoh router TCP port
    else:
        raise RuntimeError(f"Zenoh Router failed to listen on {endpoint}")

    try:
        await wait_for_zenoh_discovery(check_session, "sim/router/check")
    finally:
        await asyncio.to_thread(check_session.close)

    yield endpoint

    # Cancel the background stream readers so they don't deadlock
    for task in _router_tasks:
        task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await task

    if proc.returncode is None:
        proc.terminate()
        try:
            # Lowered to 0.5 to prevent tests from appearing "stuck" at the end
            await asyncio.wait_for(proc.wait(), timeout=0.5)
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

    # Wait for session to connect to the router by waiting for the router's liveliness token
    try:
        await wait_for_zenoh_discovery(session, "sim/router/check")
    except TimeoutError:
        await asyncio.to_thread(session.close)
        raise RuntimeError(f"Failed to connect Zenoh session to {zenoh_router}") from None

    yield session
    await asyncio.to_thread(session.close)


class VirtmcuSimulation:
    """
    Enterprise-grade simulation orchestrator.
    Handles the entire lifecycle: Bring-Up, Synchronization, and Teardown.
    """

    def __init__(self, bridges: list[QmpBridge] | QmpBridge, vta: VirtualTimeAuthority, init_barrier: bool = True):
        if isinstance(bridges, list):
            self.bridges = bridges
            self.bridge = bridges[0] if bridges else None
        else:
            self.bridges = [bridges]
            self.bridge = bridges
        self.vta = vta
        self.init_barrier = init_barrier

    async def __aenter__(self):
        # 1. Bring-Up: Start the guest emulation
        # Must be called BEFORE vta.init() because QEMU is launched with -S (suspended)
        # and won't process Zenoh queries (like the 0-ns sync) until the event loop starts.
        for b in self.bridges:
            await b.start_emulation()

        # SAFETY: Give QEMU/Plugins a tiny bit of slack to start and register Zenoh handlers
        await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: bring-up slack

        # 2. Setup: Deterministic Initialization Barrier
        # In coordinated mode, we must be ready to respond to the 0-ns GET.
        # Waits for Zenoh discovery and ensures 0-ns sync.
        if self.init_barrier:
            await self.vta.init()

        return self

    async def __aexit__(self, exc_type, exc, tb):
        # 3. Teardown: Deterministic Cleanup
        for b in self.bridges:
            await b.close()


@pytest_asyncio.fixture
async def simulation(qemu_launcher, zenoh_session, zenoh_router):
    """
    Fixture that provides a declarative simulation environment.
    Usage:
        async with await simulation(dtb, kernel) as sim:
            await sim.vta.step(1_000_000)
    """

    async def _create_sim(dtb_path, kernel_path=None, nodes=None, extra_args=None, **kwargs):
        router_endpoint = zenoh_router
        nodes = [0] if nodes is None else list(nodes)
        extra_args = [] if extra_args is None else list(extra_args)

        # Robustly inject router and standard properties into virtmcu devices
        processed_args = []
        has_clock = False

        i = 0
        while i < len(extra_args):
            arg = str(extra_args[i])
            if arg in ["-device", "-chardev", "-netdev"] and i + 1 < len(extra_args):
                val = str(extra_args[i + 1])
                if "virtmcu-clock" in val:
                    has_clock = True
                    if "router=" not in val:
                        val = f"{val},router={router_endpoint}"
                    if "node=" not in val:
                        val = f"{val},node={nodes[0]}"
                    if "mode=" not in val:
                        val = f"{val},mode=slaved-icount"

                    if "stall-timeout=" not in val:
                        base_stall = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
                        scaled_stall = int(base_stall * get_time_multiplier())
                        val = f"{val},stall-timeout={scaled_stall}"

                elif "virtmcu" in val:
                    if "router=" not in val:
                        val = f"{val},router={router_endpoint}"
                processed_args.extend([arg, val])
                i += 2
            else:
                if "virtmcu-clock" in arg:
                    has_clock = True
                    # If passed without -device, we assume caller wants it added correctly
                    if "router=" not in arg:
                        arg = f"{arg},router={router_endpoint}"
                    if "node=" not in arg:
                        arg = f"{arg},node={nodes[0]}"
                    if "mode=" not in arg:
                        arg = f"{arg},mode=slaved-icount"

                    if "stall-timeout=" not in arg:
                        base_stall = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
                        scaled_stall = int(base_stall * get_time_multiplier())
                        arg = f"{arg},stall-timeout={scaled_stall}"
                    processed_args.extend(["-device", arg])

                elif "virtmcu" in arg and arg not in ["-device", "-chardev"]:
                    if "router=" not in arg:
                        arg = f"{arg},router={router_endpoint}"
                    # Decide if it's a device or chardev based on name
                    prefix = "-chardev" if "virtmcu" in arg and "id=" in arg else "-device"
                    processed_args.extend([prefix, arg])
                else:
                    processed_args.append(arg)
                i += 1

        # If no clock was provided, add a default one for orchestration
        if not has_clock:
            base_stall = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
            scaled_stall = int(base_stall * get_time_multiplier())
            processed_args.extend(
                [
                    "-device",
                    f"virtmcu-clock,node={nodes[0]},router={router_endpoint},stall-timeout={scaled_stall},mode=slaved-icount",
                ]
            )

        # Add standard icount configuration if slaved-icount is requested
        if any("slaved-icount" in arg for arg in processed_args) and "-icount" not in processed_args:
            processed_args.extend(["-icount", "shift=0,align=off,sleep=off"])

        # Force -S (frozen) for deterministic boot synchronization
        if "-S" not in processed_args:
            processed_args.append("-S")

        # Orchestrated simulations always use a clock, so we must bypass the Isolated isolation check
        kwargs.setdefault("ignore_clock_check", True)

        bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=processed_args, **kwargs)
        vta = VirtualTimeAuthority(zenoh_session, nodes)
        return VirtmcuSimulation(bridge, vta)

    return _create_sim


@pytest_asyncio.fixture
async def time_authority(zenoh_session):
    return VirtualTimeAuthority(zenoh_session, [0])


@pytest_asyncio.fixture
async def zenoh_coordinator(zenoh_router, request):
    """
    Fixture that starts the zenoh_coordinator.
    Supports parameterization: @pytest.mark.parametrize("zenoh_coordinator", [{"nodes": 2}], indirect=True)
    """
    params = getattr(request, "param", {})
    n_nodes = params.get("nodes", 3)

    curr = Path(Path(__file__).resolve().parent)
    while str(curr) != "/" and not (curr / "tools").exists():
        curr = Path(curr).parent
    workspace_root = curr

    from tools.testing.virtmcu_test_suite.artifact_resolver import get_rust_binary_path

    coord_bin = get_rust_binary_path("zenoh_coordinator")

    # Use a lock to build once in parallel runs
    if not coord_bin.exists():
        lock_file = workspace_root / "tools/zenoh_coordinator/build.lock"
        import fcntl

        def _blocking_build():
            with lock_file.open("w") as f:
                # This blocks until the lock is acquired
                fcntl.flock(f, fcntl.LOCK_EX)
                if not coord_bin.exists():
                    logger.info("Building zenoh_coordinator...")
                    subprocess.run(
                        ["cargo", "build", "--release"],
                        cwd=(workspace_root / "tools/zenoh_coordinator"),
                        check=True,
                    )

        await asyncio.to_thread(_blocking_build)

        # Refresh location after build
        coord_bin = get_rust_binary_path("zenoh_coordinator")

    pdes = getattr(request, "param", {}).get("pdes", False)
    logger.info(f"Starting Zenoh Coordinator (nodes={n_nodes}, pdes={pdes}) connecting to {zenoh_router}...")

    cmd = [str(coord_bin), "--connect", zenoh_router, "--nodes", str(n_nodes)]
    if pdes:
        cmd.append("--pdes")

    proc = await asyncio.create_subprocess_exec(
        *cmd,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=os.environ.copy(),
    )

    # Wait for session to connect to the router
    check_config = zenoh.Config()
    check_config.insert_json5("connect/endpoints", f'["{zenoh_router}"]')
    check_config.insert_json5("mode", '"client"')
    check_session = await asyncio.to_thread(lambda: zenoh.open(check_config))
    try:
        await wait_for_zenoh_discovery(check_session, "sim/coordinator/liveliness")
    finally:
        await asyncio.to_thread(check_session.close)

    yield proc

    if proc.returncode is None:
        proc.terminate()
        try:
            # Lowered to 0.5 to prevent tests from appearing "stuck" at the end
            await asyncio.wait_for(proc.wait(), timeout=0.5)
        except TimeoutError:
            proc.kill()
            await proc.wait()


@pytest_asyncio.fixture
async def qemu_launcher(request):
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
            modified_extra = []
            for arg in extra_args:
                if "virtmcu-clock" in str(arg) and "stall-timeout" not in str(arg):
                    arg = f"{arg},stall-timeout={_stall_timeout_ms}"
                modified_extra.append(arg)
            cmd.extend(modified_extra)

        # Task 4.1b: Critical isolation constraint - standalone mode only
        if not ignore_clock_check:
            for arg in cmd:
                if "clock" in str(arg):
                    raise ValueError(
                        "clock device detected in standalone test suite. "
                        "Isolated tests must run without external clock plugins."
                    )

        logger.info(f"Launching QEMU: {' '.join(cmd)}")

        # Start the process
        proc = await asyncio.create_subprocess_exec(
            *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE, env=os.environ.copy()
        )

        captured_stdout: list[str] = []
        captured_stderr: list[str] = []

        async def _stream_output(stream, name, capture_list=None):
            try:
                while True:
                    line = await stream.readline()
                    if not line:
                        break
                    decoded = line.decode()
                    if capture_list is not None:
                        capture_list.append(decoded)
                    logger.debug(f"QEMU {name}: {decoded.strip()}")
            except Exception as e:
                logger.error(f"Error streaming {name}: {e}")

        # Task 4.2d: Stream QEMU output in background for better debuggability.
        # We store task references to prevent them from being garbage collected.
        output_tasks = [
            asyncio.create_task(_stream_output(proc.stdout, "STDOUT", captured_stdout)),
            asyncio.create_task(_stream_output(proc.stderr, "STDERR", captured_stderr)),
        ]

        # Wait for sockets to be created by QEMU.
        try:
            # Task 4.2d: Use deterministic file discovery for IPC sockets.
            # gather(wait_for_file_creation) wakes up instantly on the inotify event.
            wait_tasks = [wait_for_file_creation(qmp_sock)]
            if not has_serial:
                wait_tasks.append(wait_for_file_creation(uart_sock))

            files_task = asyncio.ensure_future(asyncio.gather(*wait_tasks))
            exit_task = asyncio.create_task(proc.wait())

            done, pending = await asyncio.wait(
                [files_task, exit_task],
                return_when=asyncio.FIRST_COMPLETED,
                timeout=10.0 * get_time_multiplier(),
            )

            for task in pending:
                task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await task

            if exit_task in done:
                # Process exited before sockets were created.
                # Drain output tasks with a small timeout to capture all error messages.
                await asyncio.wait(output_tasks, timeout=0.5)

                stdout_text = "".join(captured_stdout)
                stderr_text = "".join(captured_stderr)
                combined_output = f"STDOUT:\n{stdout_text}\nSTDERR:\n{stderr_text}"

                if (
                    "failed to open module" in stderr_text
                    or "undefined symbol" in stderr_text
                    or "not a valid device model name" in stderr_text
                ):
                    raise RuntimeError(f"QEMU Plugin Load Error (Check #[no_mangle]):\n{combined_output}")

                if (
                    "Instrumentation Mismatch Detected" in stdout_text
                    or "Instrumentation Mismatch Detected" in stderr_text
                ):
                    raise RuntimeError(f"QEMU Sanitizer Mismatch:\n{combined_output}")

                raise RuntimeError(
                    f"QEMU exited unexpectedly (rc={proc.returncode}) before sockets appeared.\n{combined_output}"
                )

            if not done:
                raise TimeoutError()

            files_task.result()

        except (TimeoutError, RuntimeError) as e:
            if proc.returncode is None:
                proc.terminate()

            # Try to drain some output if we timed out
            await asyncio.wait(output_tasks, timeout=0.2)
            stdout_text = "".join(captured_stdout)
            stderr_text = "".join(captured_stderr)

            logger.error(f"QEMU failed to start. rc={proc.returncode}\nSTDOUT: {stdout_text}\nSTDERR: {stderr_text}")
            if isinstance(e, TimeoutError):
                raise TimeoutError(
                    f"QEMU QMP/UART sockets did not appear in time.\nSTDOUT: {stdout_text}\nSTDERR: {stderr_text}"
                ) from e
            raise

        bridge = QmpBridge()
        bridge.pid = proc.pid
        try:
            # Task: Deterministic signaling
            # Extract node_id from extra_args if available
            node_id = None
            if extra_args:
                for arg in extra_args:
                    if "node=" in str(arg):
                        try:
                            # Parse node=N from something like "-device virtmcu-clock,node=1,..."
                            import re

                            match = re.search(r"node=(\d+)", str(arg))
                            if match:
                                node_id = int(match.group(1))
                                break
                        except Exception:
                            pass

            # Cleanly retrieve zenoh_session if the current test is using it
            active_zenoh_session = None
            if "zenoh_session" in request.fixturenames:
                active_zenoh_session = request.getfixturevalue("zenoh_session")

            await bridge.connect(
                str(qmp_sock),
                None if has_serial else str(uart_sock),
                zenoh_session=active_zenoh_session,
                node_id=node_id,
            )
        except Exception as e:
            # Check if QEMU died immediately after socket creation (e.g. during QMP negotiation)
            if proc.returncode is not None:
                await yield_now()
                stderr_text = "".join(captured_stderr)
                if (
                    "failed to open module" in stderr_text
                    or "undefined symbol" in stderr_text
                    or "not a valid device model name" in stderr_text
                ):
                    raise RuntimeError(f"QEMU Plugin Load Error (Check #[no_mangle]):\n{stderr_text}") from e
                raise RuntimeError(
                    f"QEMU exited unexpectedly (rc={proc.returncode}) during QMP connect.\nSTDERR: {stderr_text}"
                ) from e

            logger.error(f"QEMU failed to establish connection: {e}")
            raise e

        instance = {"proc": proc, "bridge": bridge, "tmpdir": tmpdir, "cmd": cmd, "output_tasks": output_tasks}
        instances.append(instance)
        return bridge

    yield _launch

    # Cleanup
    for inst in instances:
        try:
            # Lowered from 5.0 to 1.0 to prevent teardown hangs taking up to 10s total
            await asyncio.wait_for(inst["bridge"].close(), timeout=1.0)
        except Exception as e:
            logger.warning(f"Error closing bridge: {e}")

        # Cancel the background stream readers so they don't deadlock with communicate()
        for task in inst["output_tasks"]:
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task

        proc = inst["proc"]
        if proc.returncode is None:
            proc.terminate()
            try:
                # Lowered from 5.0 to 0.5 to prevent tests from appearing "stuck" at the end
                await asyncio.wait_for(proc.wait(), timeout=0.5)
            except TimeoutError:
                proc.kill()
                await proc.wait()

        shutil.rmtree(inst["tmpdir"], ignore_errors=True)


@pytest_asyncio.fixture
async def qmp_bridge(qemu_launcher):
    dtb = "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not Path(dtb).exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm", "minimal.dtb"], check=True)
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

    async def init(self, timeout: float = 30.0):
        """
        Deterministic Initialization Barrier.
        """
        if not self._liveliness_checked:
            liveliness_tasks = []
            for nid in self.node_ids:
                hb_topic = f"sim/clock/liveliness/{nid}"
                liveliness_tasks.append(wait_for_zenoh_discovery(self.session, hb_topic, timeout=timeout))
            await asyncio.gather(*liveliness_tasks)
            self._liveliness_checked = True

        # SAFETY: Give QEMU a tiny bit of slack to finish its internal transition to the first barrier
        await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: initialization slack

        # Perform the 0-ns sync to ensure QEMU is perfectly frozen and ready
        await self.step(0, timeout=timeout)

    async def step(self, delta_ns: int, timeout: float = 60.0) -> Any:
        res = await super().step(delta_ns, timeout)
        return res[self.node_ids[0]]


def pytest_collection_modifyitems(config, items):  # noqa: ARG001
    computed_timeout = _DEFAULT_VTA_STEP_TIMEOUT_S + 60.0
    for item in items:
        item.add_marker(pytest.mark.timeout(computed_timeout))

    if os.environ.get("VIRTMCU_USE_ASAN") == "1" or os.environ.get("VIRTMCU_USE_TSAN") == "1":
        skip_asan = pytest.mark.skip(reason="Too timing sensitive for ASan/TSan")
        for item in items:
            if "skip_asan" in item.keywords:
                item.add_marker(skip_asan)


@pytest.hookimpl(tryfirst=True, hookwrapper=True)
def pytest_runtest_makereport(item, call):  # noqa: ARG001
    outcome = yield
    rep = outcome.get_result()
    if rep.when == "call" and rep.failed and "sim_transport" in item.funcargs:
        # Check if the test has a flight recorder transport
        t = item.funcargs["sim_transport"]
        if hasattr(t, "dump_flight_recorder"):
            import json

            log_dir = Path("test-results/flight_recorder")
            log_dir.mkdir(parents=True, exist_ok=True)
            log_file = log_dir / f"{item.name}.json"
            pcap_file = log_dir / f"{item.name}.pcap"
            try:
                with log_file.open("w") as f2:
                    json.dump(t.dump_flight_recorder(), f2, indent=2)
                if hasattr(t, "dump_pcap"):
                    t.dump_pcap(pcap_file)
                logger.info(f"\n✈️ FLIGHT RECORDER DUMPED TO: {log_file} and {pcap_file}\n")
            except Exception as e:
                logger.info(f"\n❌ Failed to dump flight recorder: {e}\n")
