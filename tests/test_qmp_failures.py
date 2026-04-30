import asyncio
from pathlib import Path

import pytest
from qemu.qmp.protocol import ConnectError, StateError
from qemu.qmp.qmp_client import ExecInterruptedError


@pytest.mark.asyncio
async def test_qemu_crash_handling(qemu_launcher):
    """
    Test how the bridge handles QEMU crashing mid-execution.
    """
    workspace_root = Path(__file__).resolve().parent.parent
    dtb = workspace_root / "tests/fixtures/guest_apps/phase1/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/phase1/hello.elf"

    # Use qemu_launcher for robust process management
    bridge = await qemu_launcher(dtb, kernel, ignore_clock_check=True)

    # Verify we can connect
    assert bridge.is_connected

    try:
        # Kill QEMU
        import psutil

        try:
            qemu_proc = psutil.Process(bridge.pid)
            qemu_proc.kill()
        except psutil.NoSuchProcess:
            pass

        # Give it a tiny moment to die
        await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: yield to let OS kill process

        # Next command should fail
        with pytest.raises((ConnectError, StateError, EOFError, asyncio.IncompleteReadError, ExecInterruptedError)):
            await bridge.qmp.execute("query-status")

    finally:
        await bridge.close()
