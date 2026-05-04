"""
SOTA Test Module: test_watchdog

Context:
This module implements tests for the test_watchdog subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_watchdog.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_watchdog_fires_on_vtime_stall(simulation: Simulation, guest_app_factory: Any) -> None:  # noqa: ANN401
    from tools.testing.utils import get_time_multiplier

    app_dir = guest_app_factory("boot_arm")
    dtb_path = app_dir / "minimal.dtb"
    kernel_path = app_dir / "hello.elf"
    # We want a very short stall-timeout to trigger it quickly, but it must be
    # scaled by the environment multiplier.
    base_stall = 2000
    scaled_stall = int(base_stall * get_time_multiplier())
    extra_args = [
        "-device",
        f"virtmcu-clock,mode=slaved-suspend,stall-timeout={scaled_stall}",
    ]
    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)

    async with simulation as sim:
        assert sim.bridge is not None
        await sim.bridge.pause_emulation()
        with pytest.raises(RuntimeError, match="reported CLOCK STALL"):
            await sim.vta.step(1_000_000, timeout=10.0)  # LINT_EXCEPTION: vta_step_loop
