"""
RISC-V boot test.
Verify that RISC-V firmware boots and prints "HI RV".
"""

from __future__ import annotations

import shutil
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tests.sim_types import SimulationCreator


@pytest.mark.asyncio
async def test_riscv_boot(simulation: SimulationCreator) -> None:

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    riscv_test_dir = workspace_root / "tests/fixtures/guest_apps/boot_riscv"
    dts = riscv_test_dir / "minimal.dts"
    kernel = riscv_test_dir / "hello.elf"

    if not dts.exists() or not kernel.exists():
        import subprocess

        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_riscv"],
            check=True,
            cwd=workspace_root,
        )

    # Boot and check UART using VirtmcuSimulation
    # We pass nodes=[] to disable clock orchestration (no virtmcu-clock device)
    # as it is not yet supported on the RISC-V virt machine.
    async with await simulation(dts, kernel, nodes=[], extra_args=["-m", "512M"]) as sim:
        # In non-orchestrated mode, we don't use vta.step()
        assert sim.bridge is not None
        assert await sim.bridge.wait_for_line_on_uart("HI RV", timeout=10.0)
