"""
Verify that QMP remains responsive even when vCPU is blocked at a quantum boundary.
This ensures that BQL yielding in clock_quantum_wait works as expected.
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


async def _qmp_worker(sim: Simulation) -> None:
    assert sim.bridge is not None
    for _ in range(50):
        await sim.bridge.execute("query-status")

@pytest.mark.asyncio
async def test_bql_qmp_deadlock(simulation: Simulation, guest_app_factory: Callable[[str], Path]) -> None:

    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"
    # Use slaved-icount for deterministic boundary blocking
    extra_args = ["-icount", "shift=0,align=off,sleep=off", "-device", f"virtmcu-clock,node=0,mode=slaved-icount,router={simulation._router}"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        # 1. Advance to first boundary (0 ns sync)
        await sim.vta.step(0)  # LINT_EXCEPTION: vta_step_loop

        # 2. Issue a large step and blast QMP concurrently
        step_task = asyncio.create_task(sim.vta.step(50_000_000))
        qmp_task = asyncio.create_task(_qmp_worker(sim))

        await asyncio.gather(step_task, qmp_task)
