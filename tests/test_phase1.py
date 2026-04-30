import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_phase1_boot(qemu_launcher):
    """
    Phase 1 smoke test: Basic boot and UART output.
    Verify that the minimal kernel prints 'HI'.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    dtb = Path(workspace_root) / "tests/fixtures/guest_apps/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "tests/fixtures/guest_apps/phase1/hello.elf"

    # 1. Build if missing
    if not Path(dtb).exists() or not Path(kernel).exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/phase1"], check=True, cwd=workspace_root)

    # 2. Boot and check UART
    bridge = await qemu_launcher(dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()

    assert await bridge.wait_for_line_on_uart("HI")
