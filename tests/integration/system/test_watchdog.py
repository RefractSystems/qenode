"""
SOTA Test Module: test_watchdog

Context:
This module implements tests for the test_watchdog subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_watchdog.
"""

import asyncio
from unittest.mock import AsyncMock, patch

import pytest


def build_boot_arm_artifacts():
    import subprocess

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"], check=True)
    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_watchdog_fires_on_vtime_stall(simulation, zenoh_router):
    dtb_path, kernel_path = build_boot_arm_artifacts()
    extra_args = [
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-suspend,router={zenoh_router}",
    ]
    async with await simulation(dtb_path, kernel_path, extra_args=extra_args) as sim:
        # Mock the get_virtual_time_ns so it appears to be stalled at 1_000_000
        with patch.object(sim.bridge, "get_virtual_time_ns", new_callable=AsyncMock, return_value=1_000_000):
            try:
                await asyncio.sleep(15.0)  # SLEEP_EXCEPTION: waiting for watchdog
            except asyncio.CancelledError as e:
                assert "Guest OS deadlocked" in str(e)
                return

    pytest.fail("Watchdog did not fire!")
