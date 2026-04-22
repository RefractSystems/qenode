from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_riscv_boot(qemu_launcher):
    """
    Phase 11: RISC-V boot test.
    Verify that RISC-V firmware boots and prints "HI RV".
    """
    workspace_root = Path(__file__).resolve().parent.parent
    riscv_test_dir = workspace_root / "test/riscv"
    dts = riscv_test_dir / "minimal.dts"
    kernel = riscv_test_dir / "hello.elf"

    if not dts.exists() or not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "test/riscv"], check=True, cwd=workspace_root)

    bridge = await qemu_launcher(dts, kernel)
    await bridge.start_emulation()

    assert await bridge.wait_for_line_on_uart("HI RV", timeout=10.0)
