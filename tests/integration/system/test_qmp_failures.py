"""
SOTA Test Module: test_qmp_failures

Context:
This module implements tests for the test_qmp_failures subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_qmp_failures.
"""

import asyncio

import pytest
from qemu.qmp.protocol import ConnectError, StateError
from qemu.qmp.qmp_client import ExecInterruptedError


@pytest.mark.asyncio
async def test_qemu_crash_handling(qemu_launcher):
    """
    Test how the bridge handles QEMU crashing mid-execution.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    # Use qemu_launcher for robust process management
    bridge = await qemu_launcher(dtb, kernel, ignore_clock_check=True)

    # Verify we can connect
    assert bridge.is_connected

    try:
        # Kill QEMU
        import psutil

        try:
            qemu_proc = psutil.Process(bridge.pid)
            qemu_proc.kill()
        except psutil.NoSuchProcess:
            pass

        # Give it a tiny moment to die
        await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: yield to let OS kill process

        # Next command should fail
        with pytest.raises((ConnectError, StateError, EOFError, asyncio.IncompleteReadError, ExecInterruptedError)):
            await bridge.qmp.execute("query-status")

    finally:
        import contextlib

        with contextlib.suppress(EOFError, ConnectionResetError, Exception):
            await bridge.close()
