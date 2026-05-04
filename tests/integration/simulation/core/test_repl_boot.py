"""
smoke test: repl2qemu parser.
Verify that a .repl file can be translated to DTB and booted.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_repl2qemu(simulation: Simulation, tmp_path: Path, guest_app_factory: Any) -> None:  # noqa: ANN401
    from tools.testing.virtmcu_test_suite.factory import compile_repl

    app_dir_yaml = guest_app_factory("yaml_boot")
    repl_file = app_dir_yaml / "test_board.repl"
    out_dtb = tmp_path / "test_board_out.dtb"

    app_dir_boot = guest_app_factory("boot_arm")
    kernel = app_dir_boot / "hello.elf"

    # 1. Build kernel (handled by guest_app_factory)

    # 2. Run parser
    compile_repl(repl_file, out_dtb)

    assert out_dtb.exists()

    # 2. Boot and check UART using Simulation
    simulation.add_node(node_id=0, dtb=out_dtb, kernel=kernel)
    async with simulation as sim:
        await sim.vta.step(100_000_000)  # LINT_EXCEPTION: vta_step_loop
        assert sim.bridge is not None
        assert await sim.bridge.wait_for_line_on_uart("HI")
