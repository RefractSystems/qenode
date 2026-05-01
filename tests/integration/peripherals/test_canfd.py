"""
SOTA Test Module: test_canfd

Context:
This module implements tests for the test_canfd subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_canfd.
"""

import logging
import os

import pytest

from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_canfd_plugin_loads():
    env = os.environ.copy()

    # We must run it via run.sh to get module paths right
    cmd = [
        "bash",
        "scripts/run.sh",
        "--dtb",
        "tests/fixtures/guest_apps/boot_arm/minimal.dtb",
        "-object",
        "can-bus,id=canbus0",
        "-object",
        "can-host-virtmcu,id=canhost0,canbus=canbus0,node=test_node,router=,topic=sim/can",
        "-monitor",
        "none",
        "-serial",
        "none",
        "-nographic",
        "-display",
        "none",
        "-S",
    ]

    async with AsyncManagedProcess(*cmd, env=env) as proc:
        try:
            await proc.wait(timeout=2.0)
        except TimeoutError:
            # QEMU shouldn't exit if it's running successfully with -S
            assert True
        else:
            logger.info(f"STDOUT: {proc.stdout_text}")
            logger.info(f"STDERR: {proc.stderr_text}")
            assert proc.returncode == 0, f"QEMU crashed or failed to load the plugin. STDERR: {proc.stderr_text}"
