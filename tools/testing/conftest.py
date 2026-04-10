import asyncio
import logging
import os
import shutil
import subprocess
import tempfile

import pytest_asyncio

from tools.testing.qmp_bridge import QmpBridge

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

@pytest_asyncio.fixture
async def qemu_launcher():
    """
    Fixture that returns a function to launch QEMU instances.
    Ensures all instances are cleaned up after the test.
    """
    instances = []

    async def _launch(dtb_path, kernel_path=None, extra_args=None):
        # Create a unique temporary directory for this QEMU instance
        tmpdir = tempfile.mkdtemp(prefix="virtmcu-test-")
        qmp_sock = os.path.join(tmpdir, "qmp.sock")
        uart_sock = os.path.join(tmpdir, "uart.sock")

        # Build the command using run.sh
        # We use absolute paths to ensure it works from any directory
        workspace_root = os.getcwd()
        run_script = os.path.join(workspace_root, "scripts/run.sh")

        cmd = [run_script, "--dtb", os.path.abspath(dtb_path)]
        if kernel_path:
            cmd.extend(["--kernel", os.path.abspath(kernel_path)])

        # Add QMP and UART sockets
        # Note: we use 'server,nowait' because QEMU should start and wait for us
        cmd.extend([
            "-qmp", f"unix:{qmp_sock},server,nowait",
            "-serial", f"unix:{uart_sock},server,nowait",
            "-display", "none",
            "-nographic"
        ])

        if extra_args:
            cmd.extend(extra_args)

        # Task 4.1b: Critical isolation constraint - standalone mode only
        for arg in cmd:
            if "zenoh-clock" in arg:
                raise ValueError("zenoh-clock device detected in standalone test suite. "
                                 "Phase 4 tests must run without external clock plugins.")

        logger.info(f"Launching QEMU: {' '.join(cmd)}")

        # Start the process
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=os.environ.copy()
        )

        # Wait for sockets to be created by QEMU.
        # Poll every 100 ms (up to 10 s). Also check for premature exit.
        retries = 100
        while retries > 0:
            if proc.returncode is not None:
                # QEMU exited before sockets appeared — capture output and fail fast.
                stdout, stderr = await proc.communicate()
                raise RuntimeError(
                    f"QEMU exited unexpectedly (rc={proc.returncode}) before sockets appeared.\n"
                    f"STDOUT: {stdout.decode()}\nSTDERR: {stderr.decode()}"
                )
            if os.path.exists(qmp_sock) and os.path.exists(uart_sock):
                break
            await asyncio.sleep(0.1)
            retries -= 1
        else:
            # 10 s elapsed — kill the process and drain its output.
            proc.terminate()
            stdout, stderr = await proc.communicate()
            logger.error(f"QEMU failed to start. STDOUT: {stdout.decode()} STDERR: {stderr.decode()}")
            raise TimeoutError("QEMU QMP/UART sockets did not appear in time")

        bridge = QmpBridge()
        await bridge.connect(qmp_sock, uart_sock)

        instance = {
            "proc": proc,
            "bridge": bridge,
            "tmpdir": tmpdir
        }
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
            except asyncio.TimeoutError:
                proc.kill()
                await proc.wait()

        shutil.rmtree(inst["tmpdir"], ignore_errors=True)

@pytest_asyncio.fixture
async def qmp_bridge(qemu_launcher):
    """
    A convenience fixture that launches a default QEMU instance with
    the phase1 minimal DTB and hello.elf firmware.

    Uses -S to start paused, connects, and then resumes to ensure
    that early firmware output is captured.
    """
    dtb = "test/phase1/minimal.dtb"
    kernel = "test/phase1/hello.elf"

    # Ensure DTB exists
    if not os.path.exists(dtb):
        # Try to build it if missing
        subprocess.run(["make", "-C", "test/phase1", "minimal.dtb"], check=True)

    bridge = await qemu_launcher(dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()
    return bridge
