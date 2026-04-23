import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_phase3_repl2qemu(qemu_launcher, tmp_path):
    """
    Phase 3 smoke test: repl2qemu parser.
    Verify that a .repl file can be translated to DTB and booted.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    repl_file = Path(workspace_root) / "test/phase3/test_board.repl"
    out_dtb = tmp_path / "test_board_out.dtb"
    kernel = Path(workspace_root) / "test/phase1/hello.elf"

    # 1. Build kernel if missing
    if not Path(kernel).exists():
        subprocess.run(["make", "-C", "test/phase1"], check=True, cwd=workspace_root)

    # 2. Run parser
    subprocess.run(
        ["python3", "-m", "tools.repl2qemu", repl_file, "--out-dtb", out_dtb], check=True, cwd=workspace_root
    )

    assert Path(out_dtb).exists()

    # 2. Boot and check UART
    bridge = await qemu_launcher(out_dtb, kernel, extra_args=["-S"])
    await bridge.start_emulation()

    assert await bridge.wait_for_line_on_uart("HI")
