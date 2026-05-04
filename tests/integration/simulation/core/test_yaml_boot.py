"""
YAML platform boot test.
Verify that a platform defined in YAML can boot and print "HI".
"""

from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_yaml_platform_boot(
    simulation: Simulation, tmp_path: Path, guest_app_factory: Callable[[str], Path]
) -> None:
    from tools.testing.env import WORKSPACE_ROOT
    from tools.testing.virtmcu_test_suite.factory import compile_yaml

    workspace_root = WORKSPACE_ROOT
    yaml_file = workspace_root / "tests/fixtures/guest_apps/yaml_boot/test_board.yaml"
    
    app_dir = guest_app_factory("boot_arm")
    kernel = app_dir / "hello.elf"

    dtb = tmp_path / "test_board.dtb"
    compile_yaml(yaml_file, dtb)

    # Boot and check UART using Simulation
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel)
    async with simulation as sim:
        await sim.vta.step(100_000_000)  # LINT_EXCEPTION: vta_step_loop
        assert sim.bridge is not None
        assert await sim.bridge.wait_for_line_on_uart("HI", timeout=5.0)
