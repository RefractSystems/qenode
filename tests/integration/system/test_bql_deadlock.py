"""
Verify that QMP remains responsive even when vCPU is blocked at a quantum boundary.
This ensures that BQL yielding in clock_quantum_wait works as expected.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


def build_bql_deadlock_artifacts() -> tuple[Path, Path]:
    import subprocess

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(
            [shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"],
            check=True,
            cwd=workspace_root,
        )

    return dtb_path, kernel_path


async def _qmp_worker(sim: Simulation) -> None:
    assert sim.bridge is not None
    for _ in range(10):
        await sim.bridge.execute("query-status")
        await asyncio.sleep(0.05)  # SLEEP_EXCEPTION: yield for QMP throughput


@pytest.mark.asyncio
async def test_bql_qmp_deadlock(simulation: Simulation) -> None:

    dtb, kernel = build_bql_deadlock_artifacts()
    # Use slaved-icount for deterministic boundary blocking
    extra_args = ["-icount", "shift=0,align=off,sleep=off", "-device", "virtmcu-clock,node=0,mode=slaved-icount"]

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        # 1. Advance to first boundary (0 ns sync)
        await sim.vta.step(0)

        # 2. Start a background task that constantly queries QMP
        worker_task = asyncio.create_task(_qmp_worker(sim))

        # 3. Perform several clock steps.
        # Between steps, QEMU will be blocked in clock_quantum_wait (yielding BQL).
        for _ in range(5):
            await sim.vta.step(1_000_000)
            # Give QMP worker a chance to run while we are at the boundary
            await asyncio.sleep(0.1)  # SLEEP_EXCEPTION: yield for background worker

        await worker_task
