"""
SOTA Test Module: test_boot_arm

Context:
This module implements basic boot and initialization tests for the ARM generic machine.

Objective:
Verify that the `arm-generic-fdt` machine can successfully boot a minimal ELF payload,
execute the primary boot sequence, and transmit deterministic output over the UART.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

from tools.testing.env import build_guest_app

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_boot_arm(simulation: Simulation) -> None:

    # 1. Autonomously resolve paths and build the guest firmware
    app_dir = build_guest_app("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    # 2. Boot and check UART using Simulation
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel)
    async with simulation as sim:
        # Advance clock and wait for UART deterministically
        await sim.node(0).wait_for_uart("HI", timeout_ns=1_000_000_000, step_ns=10_000_000)
