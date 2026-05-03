"""
YAML platform boot test.
Verify that a platform defined in YAML can boot and print "HI".
"""

from __future__ import annotations

import shutil
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_yaml_platform_boot(simulation: Simulation, tmp_path: Path) -> None:

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_file = workspace_root / "tests/fixtures/guest_apps/yaml_boot/test_board.yaml"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not kernel.exists():
        import subprocess

        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root
        )

    dtb = tmp_path / "test_board.dtb"
    import subprocess

    subprocess.run(
        [shutil.which("uv") or "uv", "run", "python3", "-m", "tools.yaml2qemu", str(yaml_file), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

    # Boot and check UART using Simulation
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel)
    async with simulation as sim:
        await sim.vta.step(100_000_000)
        assert sim.bridge is not None
        assert await sim.bridge.wait_for_line_on_uart("HI", timeout=5.0)
