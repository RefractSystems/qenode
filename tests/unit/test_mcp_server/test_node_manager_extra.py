"""
SOTA Test Module: test_node_manager_extra

Context:
This module implements tests for the test_node_manager_extra subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_node_manager_extra.
"""

from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from tools.mcp_server.node_manager import NodeManager


@pytest.fixture
def manager():
    return NodeManager()


@pytest.mark.asyncio
async def test_provision_board_invalid_yaml(manager):
    with pytest.raises(ValueError, match="Invalid board configuration"):
        await manager.provision_board("bad_node", "this is: { not yaml", "yaml")


@pytest.mark.asyncio
async def test_provision_board_valid_yaml(manager):
    # Minimal valid YAML for virtmcu
    valid_yaml = """
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals: []
"""
    # We need to mock tools.yaml2qemu.main to avoid full execution if we want a pure unit test,
    # but here we might want to actually call it if dependencies are met.
    # Given the previous tests, it seems it's better to mock it for unit tests.
    with patch("tools.yaml2qemu.main") as mock_main:
        await manager.provision_board("good_node", valid_yaml, "yaml")
        assert Path(manager.get_node("good_node").yaml_path).exists()
        mock_main.assert_called_once()


@pytest.mark.asyncio
async def test_start_node_already_running(manager):
    node = manager.get_node("node0")
    node.process = MagicMock()
    node.process.returncode = None
    node.yaml_path = "/tmp/dummy.yaml"

    with pytest.raises(RuntimeError, match="already running"):
        await manager.start_node("node0")


@pytest.mark.asyncio
async def test_start_node_no_provision(manager):
    with pytest.raises(RuntimeError, match="has not been provisioned"):
        await manager.start_node("node0")


@pytest.mark.asyncio
async def test_flash_firmware_not_found(manager):
    with pytest.raises(FileNotFoundError):
        manager.flash_firmware("node0", "/non/existent/path")


@pytest.mark.asyncio
async def test_stop_node(manager):
    node = manager.get_node("node0")
    node.process = AsyncMock()
    node.process.returncode = None
    node.process.terminate = MagicMock()
    node.process.wait = AsyncMock()

    await manager.stop_node("node0")
    node.process.terminate.assert_called_once()
    node.process.wait.assert_called_once()
