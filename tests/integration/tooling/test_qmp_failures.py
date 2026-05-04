#
# Copyright (C) 2026 Refract Systems
#
# This file is part of VirtMCU.
#
# Ensure correct functionality, performance, and deterministic execution of test_qemu_crash_handling.

from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


@pytest.mark.asyncio
async def test_qemu_crash_handling(simulation: Simulation, tmp_path: Path) -> None:
    """
    Test how the bridge handles QEMU crashing mid-execution.
    """
    import psutil

    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    # Use simulation for robust process management
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=None)
    async with simulation as sim:
        # Verify we can connect
        bridge = sim.bridge
        assert bridge is not None
        assert bridge.is_connected
        pid = bridge.pid

        # Kill QEMU and children surgically
        try:
            parent = psutil.Process(pid)
            for child in parent.children(recursive=True):
                child.kill()
            parent.kill()
        except psutil.NoSuchProcess:
            pass

        # Next operation should eventually fail
        # We use a helper to poll until it fails to satisfy PT012
        async def poll_until_fail() -> None:
            for _ in range(20):
                await bridge.execute("query-status")
                await asyncio.sleep(0.1)  # SLEEP_EXCEPTION: waiting for OS to reclaim resources

        with pytest.raises(Exception, match=".*"):
            await poll_until_fail()
