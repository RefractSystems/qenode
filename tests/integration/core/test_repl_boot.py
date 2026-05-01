import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_repl2qemu(simulation, tmp_path):
    """
    smoke test: repl2qemu parser.
    Verify that a .repl file can be translated to DTB and booted.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    repl_file = Path(workspace_root) / "tests/fixtures/guest_apps/yaml_boot/test_board.repl"
    out_dtb = tmp_path / "test_board_out.dtb"
    kernel = Path(workspace_root) / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    # 1. Build kernel if missing
    if not Path(kernel).exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root)

    # 2. Run parser
    subprocess.run(
        ["python3", "-m", "tools.repl2qemu", repl_file, "--out-dtb", out_dtb], check=True, cwd=workspace_root
    )

    assert Path(out_dtb).exists()

    # 2. Boot and check UART using VirtmcuSimulation
    async with await simulation(out_dtb, kernel) as sim:
        await sim.vta.step(100_000_000)
        assert await sim.bridge.wait_for_line_on_uart("HI")
