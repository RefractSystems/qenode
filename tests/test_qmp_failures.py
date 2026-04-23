import asyncio
from pathlib import Path

import pytest
from qemu.qmp.protocol import ConnectError, StateError


@pytest.mark.asyncio
async def test_qemu_crash_handling(qemu_launcher):
    """
    Test how the bridge handles QEMU crashing mid-execution.
    """
    workspace_root = Path(__file__).resolve().parent.parent
    dtb = workspace_root / "test/phase1/minimal.dtb"
    kernel = workspace_root / "test/phase1/hello.elf"

    # Use qemu_launcher for robust process management
    bridge = await qemu_launcher(dtb, kernel, ignore_clock_check=True)

    # Verify we can connect
    assert bridge.qmp.is_connected()

    try:
        # Kill QEMU
        # qemu_launcher doesn't expose proc easily, but we can find it
        import psutil
        qemu_proc = None
        for p in psutil.process_iter(["cmdline"]):
            if "qemu-system-arm" in (p.info["cmdline"] or []) and str(dtb) in str(p.info["cmdline"]):
                qemu_proc = p
                break

        assert qemu_proc is not None
        qemu_proc.kill()

        # Give it a tiny moment to die
        await asyncio.sleep(0.5)

        # Next command should fail
        with pytest.raises((ConnectError, StateError, EOFError, asyncio.IncompleteReadError)):
            await bridge.qmp.execute("query-status")

    finally:
        await bridge.close()
