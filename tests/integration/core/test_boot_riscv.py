"""
RISC-V boot test.
Verify that RISC-V firmware boots and prints "HI RV".
"""

from __future__ import annotations

import shutil
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_riscv_boot(simulation: Simulation) -> None:

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    riscv_test_dir = workspace_root / "tests/fixtures/guest_apps/boot_riscv"
    dtb = riscv_test_dir / "minimal.dtb"
    kernel = riscv_test_dir / "hello.elf"

    if not dtb.exists() or not kernel.exists():
        import subprocess

        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_riscv"],
            check=True,
            cwd=workspace_root,
        )

    # Boot and check UART using Simulation
    simulation.add_node(
        node_id=0,
        dtb=dtb,
        kernel=kernel,
        extra_args=["-m", "512M", "--arch", "riscv64", "-bios", "none"],
    )
    async with simulation as sim:
        assert sim.bridge is not None
        assert sim.vta is not None

        # In orchestrated mode, we need to drive the clock
        import asyncio

        async def step_clock() -> None:
            for _ in range(100):
                await sim.vta.step(delta_ns=10000000)  # 10ms steps

        # Run clock in background while we wait for UART
        clock_task = asyncio.create_task(step_clock())

        assert await sim.bridge.wait_for_line_on_uart("HI RV", timeout=10.0)

        clock_task.cancel()
