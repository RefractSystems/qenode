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
        # Advance clock to allow boot (up to 1s in virtual time)
        success = False
        for _ in range(100):  # 100 * 10ms = 1s
            await sim.vta.step(10_000_000)
            assert sim.bridge is not None
            if await sim.bridge.wait_for_line_on_uart("HI", timeout=0.01):
                success = True
                break

        if not success:
            await sim.bridge.get_virtual_time_ns()
        assert success
