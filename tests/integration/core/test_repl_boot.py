"""
smoke test: repl2qemu parser.
Verify that a .repl file can be translated to DTB and booted.
"""

from __future__ import annotations

import shutil
import subprocess
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_repl2qemu(simulation: Simulation, tmp_path: Path) -> None:

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    repl_file = workspace_root / "tests/fixtures/guest_apps/yaml_boot/test_board.repl"
    out_dtb = tmp_path / "test_board_out.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    # 1. Build kernel if missing
    if not kernel.exists():
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root
        )

    # 2. Run parser
    subprocess.run(
        [shutil.which("python3") or "python3", "-m", "tools.repl2qemu", repl_file, "--out-dtb", out_dtb],
        check=True,
        cwd=workspace_root,
    )

    assert out_dtb.exists()

    # 2. Boot and check UART using Simulation
    simulation.add_node(node_id=0, dtb=out_dtb, kernel=kernel)
    async with simulation as sim:
        await sim.vta.step(100_000_000)
        assert sim.bridge is not None
        assert await sim.bridge.wait_for_line_on_uart("HI")
