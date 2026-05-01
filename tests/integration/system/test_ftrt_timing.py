"""
SOTA Test Module: test_ftrt

Context:
This module implements tests for the test_ftrt subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_ftrt.
"""

import asyncio
import logging
import os

import pytest

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_faster_than_real_time(simulation, zenoh_router):
    """
    Proves that the simulation runs Faster-Than-Real-Time (FTRT)
    when pacing is disabled (multiplier = 0.0), unbound by pseudo-polling bottlenecks.
    """
    import subprocess

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    if not kernel_path.exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm"], check=True, cwd=workspace_root)

    # Launch node in slaved-icount mode so we strictly govern its virtual clock
    extra_args = [
        "-device",
        f"virtmcu-clock,node=1,mode=slaved-icount,router={zenoh_router}",
    ]

    # Do NOT run this test under ASan/TSan or Miri where FTRT is impossible
    if os.environ.get("VIRTMCU_USE_ASAN") == "1" or os.environ.get("VIRTMCU_USE_TSAN") == "1":
        pytest.skip("ASan/TSan overhead inherently prevents Faster-Than-Real-Time execution.")

    async with await simulation(dtb_path, kernel_path, nodes=[1], extra_args=extra_args) as sim:
        loop = asyncio.get_running_loop()
        start_wall = loop.time()

        # Step exactly 20 seconds of virtual time
        target_virtual_ns = 20_000_000_000

        # Chunk the execution into 100ms blocks
        chunk_ns = 100_000_000
        for _ in range(target_virtual_ns // chunk_ns):
            await sim.vta.step(chunk_ns, timeout=10.0)

        end_wall = loop.time()
        elapsed_wall = end_wall - start_wall

        logger.info(f"Executed 20.0s of Virtual Time in {elapsed_wall:.2f}s of Wall-Clock Time.")

        # Assert FTRT efficiency: 20s of virtual time MUST complete in < 5 seconds of real time.
        assert elapsed_wall < 5.0, (
            f"FTRT failed! Took {elapsed_wall}s to simulate 20s. Framework is likely bottlenecking execution."
        )
