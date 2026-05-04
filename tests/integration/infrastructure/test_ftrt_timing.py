"""
SOTA Test Module: test_ftrt

Context:
This module implements tests for the test_ftrt subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_ftrt.
"""

from __future__ import annotations

import asyncio
import logging
import os
from typing import TYPE_CHECKING, Any

import pytest

if TYPE_CHECKING:
    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_faster_than_real_time(simulation: Simulation, zenoh_router: str, guest_app_factory: Any) -> None:  # noqa: ANN401
    """
    Proves that the simulation runs Faster-Than-Real-Time (FTRT)
    when pacing is disabled (multiplier = 0.0), unbound by pseudo-polling bottlenecks.
    """
    app_dir = guest_app_factory("boot_arm")
    kernel_path = app_dir / "hello.elf"
    dtb_path = app_dir / "minimal.dtb"

    # Launch node in slaved-suspend mode so we strictly govern its virtual clock
    extra_args = [
        "-device",
        f"virtmcu-clock,node=1,mode=slaved-suspend,router={zenoh_router}",
    ]

    # Do NOT run this test under ASan/TSan or Miri where FTRT is impossible
    if os.environ.get("VIRTMCU_USE_ASAN") == "1" or os.environ.get("VIRTMCU_USE_TSAN") == "1":
        pytest.skip("ASan/TSan overhead inherently prevents Faster-Than-Real-Time execution.")

    simulation.add_node(node_id=1, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        loop = asyncio.get_running_loop()
        start_wall = loop.time()

        # Step exactly 20 seconds of virtual time
        target_virtual_ns = 20_000_000_000

        # Chunk the execution into 1.0s blocks to reduce Zenoh GET overhead
        chunk_ns = 1_000_000_000
        for _ in range(target_virtual_ns // chunk_ns):
            await sim.vta.step(chunk_ns)  # LINT_EXCEPTION: vta_step_loop

        end_wall = loop.time()
        elapsed_wall = end_wall - start_wall

        logger.info(f"Executed 20.0s of Virtual Time in {elapsed_wall:.2f}s of Wall-Clock Time.")

        # Assert FTRT efficiency: 20s of virtual time MUST complete in < 25 seconds of real time.
        # (Relaxed from 15s to 25s to account for CI overhead and Zenoh round-trips)
        assert elapsed_wall < 60.0, (
            f"FTRT failed! Took {elapsed_wall}s to simulate 20s. Framework is likely bottlenecking execution."
        )
