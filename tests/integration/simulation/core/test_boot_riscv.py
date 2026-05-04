"""
RISC-V boot test.
Verify that RISC-V firmware boots and prints "HI RV".
"""

from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_riscv_boot(simulation: Simulation, guest_app_factory: Callable[[str], Path]) -> None:

    app_dir = guest_app_factory("boot_riscv")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    # Boot and check UART using Simulation
    simulation.add_node(
        node_id=0,
        dtb=dtb,
        kernel=kernel,
        extra_args=["-m", "512M", "--arch", "riscv64", "-bios", "none"],
    )
    async with simulation as sim:
        # Wait for UART deterministically
        await sim.node(0).wait_for_uart("HI RV", timeout_ns=1_000_000_000, step_ns=10_000_000)
