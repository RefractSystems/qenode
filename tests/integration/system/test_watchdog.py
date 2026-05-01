"""
SOTA Test Module: test_watchdog

Context:
This module implements tests for the test_watchdog subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_watchdog.
"""

from __future__ import annotations

import asyncio
import shutil
from pathlib import Path
from typing import TYPE_CHECKING
from unittest.mock import AsyncMock, patch

import pytest

if TYPE_CHECKING:
    from tests.sim_types import SimulationCreator


def build_boot_arm_artifacts() -> tuple[Path, Path]:
    import subprocess

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"
    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run([shutil.which("make") or "make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"], check=True)
    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_watchdog_fires_on_vtime_stall(simulation: SimulationCreator, zenoh_router: str) -> None:
    dtb_path, kernel_path = build_boot_arm_artifacts()
    extra_args = [
        "-device",
        f"virtmcu-clock,node=0,mode=slaved-suspend,router={zenoh_router}",
    ]
    async with await simulation(dtb_path, kernel_path, extra_args=extra_args) as sim:
        # Mock the get_virtual_time_ns so it appears to be stalled at 1_000_000
        with patch.object(sim.bridge, "get_virtual_time_ns", new_callable=AsyncMock, return_value=1_000_000):
            with pytest.raises(asyncio.CancelledError, match="Guest OS deadlocked"):
                await asyncio.sleep(45.0)  # SLEEP_EXCEPTION: waiting for watchdog
