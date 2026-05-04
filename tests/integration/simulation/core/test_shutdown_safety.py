from __future__ import annotations

import asyncio
import logging
from typing import TYPE_CHECKING, Any

import pytest

from tools.testing.utils import yield_now

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_shutdown_while_blocked(simulation: Simulation, guest_app_factory: Any) -> None:  # noqa: ANN401
    """
    Spawns a simulation, starts it, then immediately stops it to catch teardown races.
    """
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=None)
    async with simulation as sim:
        # Give it a few ms to boot deterministically
        await sim.vta.step(5_000_000)  # LINT_EXCEPTION: vta_step_loop

    # Simulation context exit should be clean
    logger.info("Simulation closed successfully")


@pytest.mark.asyncio
async def test_shutdown_during_vta_step(simulation: Simulation, guest_app_factory: Any) -> None:  # noqa: ANN401
    """
    Stress the teardown by closing the transport while a VTA step is in flight.
    """
    app_dir = guest_app_factory("boot_arm")
    dtb = app_dir / "minimal.dtb"
    kernel = app_dir / "hello.elf"

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=None)

    # We will manually tear down inside the block so we can catch the exception
    async with simulation as sim:
        # Start a long step in background
        step_task = asyncio.create_task(sim.vta.step(10_000_000_000))

        # Give it a tiny bit to actually start and send the request
        await yield_now()

        # Shutdown while step is in flight
        logger.info("Closing simulation while VTA step is in flight...")
        if sim.transport:
            await sim.transport.stop()

    try:
        await step_task
    except Exception as e:  # noqa: BLE001
        # During shutdown, various communication errors are expected as the session closes
        logger.info(f"Step task failed as expected during shutdown: {e}")
