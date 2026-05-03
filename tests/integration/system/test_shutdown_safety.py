from __future__ import annotations

import asyncio
import logging
import shutil
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


"""
SOTA Test Module: test_shutdown_safety

Context:
This module verifies that the simulation shuts down cleanly without UAF or hangs.

Objective:
Ensure that stopping a simulation while MMIO or Clock ops are pending is safe.
"""

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_shutdown_while_blocked(simulation: Simulation) -> None:
    """
    Spawns a simulation, starts it, then immediately stops it to catch teardown races.
    """
    import subprocess

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not dtb.exists():
        subprocess.run([shutil.which("make") or "make", "-C", str(dtb.parent), "all"], check=True)

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=None)
    async with simulation:
        # Give it a few ms to boot
        await asyncio.sleep(0.05)  # SLEEP_EXCEPTION: booting

    # Simulation context exit should be clean
    logger.info("Simulation closed successfully")


@pytest.mark.asyncio
async def test_shutdown_during_vta_step(simulation: Simulation) -> None:
    """
    Stress the teardown by closing the transport while a VTA step is in flight.
    """
    import subprocess

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not dtb.exists():
        subprocess.run([shutil.which("make") or "make", "-C", str(dtb.parent), "all"], check=True)

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=None)
    async with simulation as sim:
        # Start a long step in background
        step_task = asyncio.create_task(sim.vta.step(100_000_000))

        # Wait a tiny bit for it to send the query
        await asyncio.sleep(0.01)  # SLEEP_EXCEPTION: scheduling

        # Now context exit will happen while task is pending

    try:
        await step_task
    except Exception as e:  # noqa: BLE001
        # During shutdown, various communication errors are expected as the session closes
        logger.info(f"Step task failed as expected during shutdown: {e}")
