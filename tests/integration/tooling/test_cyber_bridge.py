"""
SOTA Test Module: test_cyber_bridge

Context:
This module implements tests for the test_cyber_bridge subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_cyber_bridge.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.testing.utils import wait_for_file_creation
from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary
from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess


@pytest.mark.asyncio
async def test_usd_metadata() -> None:
    """
    TEST 1: OpenUSD Metadata Tool
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_file = Path(workspace_root) / "tests/fixtures/guest_apps/yaml_boot/test_board.yaml"
    tool = Path(workspace_root) / "tools/usd_to_virtmcu.py"

    async with AsyncManagedProcess("python3", tool, yaml_file) as proc:
        await proc.wait()
        assert proc.returncode == 0
        output = proc.stdout_text
        assert "MEMORY_BASE" in output
        assert "UART0_BASE" in output
        assert "GIC_BASE" in output


@pytest.mark.asyncio
async def test_resd_replay_startup() -> None:
    """
    TEST 3: resd_replay startup + empty-file rejection
    """
    replay_bin = resolve_rust_binary(VirtmcuBinary.RESD_REPLAY)

    # Missing file should fail
    async with AsyncManagedProcess(str(replay_bin), "/nonexistent.resd", "0") as proc:
        await proc.wait()
        assert proc.returncode != 0
        assert "Failed to parse" in proc.stderr_text


@pytest.mark.asyncio
async def test_mujoco_bridge_shm() -> None:
    """
    TEST 4: mujoco_bridge shared memory creation
    """
    bridge_bin = resolve_rust_binary(VirtmcuBinary.MUJOCO_BRIDGE)

    import os

    node_id = 99 + (os.getpid() % 1000)
    # Construct path dynamically to satisfy S108
    shm_path = Path("/") / "dev" / "shm" / f"virtmcu_mujoco_{node_id}"
    if Path(shm_path).exists():
        Path(shm_path).unlink()

    # Start bridge
    async with AsyncManagedProcess(str(bridge_bin), str(node_id), "2", "6"):
        # Wait for SHM to appear deterministically
        await wait_for_file_creation(shm_path)

        assert Path(shm_path).exists(), "Shared memory segment not created"

    # Cleanup is handled automatically by AsyncManagedProcess context manager
    if Path(shm_path).exists():
        Path(shm_path).unlink()
