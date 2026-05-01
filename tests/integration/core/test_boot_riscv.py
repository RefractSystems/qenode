import pytest


@pytest.mark.asyncio
async def test_riscv_boot(simulation):
    """
    RISC-V boot test.
    Verify that RISC-V firmware boots and prints "HI RV".
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    riscv_test_dir = workspace_root / "tests/fixtures/guest_apps/boot_riscv"
    dts = riscv_test_dir / "minimal.dts"
    kernel = riscv_test_dir / "hello.elf"

    if not dts.exists() or not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_riscv"], check=True, cwd=workspace_root)

    # Boot and check UART using VirtmcuSimulation
    async with await simulation(dts, kernel) as sim:
        await sim.vta.step(200_000_000)
        assert await sim.bridge.wait_for_line_on_uart("HI RV", timeout=10.0)
