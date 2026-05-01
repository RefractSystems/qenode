"""
SOTA Test Module: test_node_manager

Context:
This module implements tests for the test_node_manager subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_node_manager.
"""

from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

from tools.mcp_server.node_manager import NodeManager


@pytest.fixture
def node_manager():
    return NodeManager()


def test_get_node(node_manager):
    node = node_manager.get_node("node0")
    assert node.node_id == "node0"
    assert "node0" in node_manager.nodes


@pytest.mark.asyncio
async def test_provision_board(node_manager):
    node_id = "test_node"
    config = "machine: { name: test }"
    await node_manager.provision_board(node_id, config)
    node = node_manager.get_node(node_id)
    assert node.yaml_path is not None
    assert Path(node.yaml_path).exists()
    with Path(node.yaml_path).open() as f:
        assert f.read() == config
    Path(node.yaml_path).unlink()


def test_flash_firmware(node_manager):
    node_id = "test_node"
    # Create a dummy file
    with Path("/tmp/dummy.elf").open("w") as f:
        f.write("dummy")

    node_manager.flash_firmware(node_id, "/tmp/dummy.elf")
    node = node_manager.get_node(node_id)
    assert node.firmware_path == "/tmp/dummy.elf"

    with pytest.raises(FileNotFoundError):
        node_manager.flash_firmware(node_id, "/tmp/nonexistent.elf")

    Path("/tmp/dummy.elf").unlink()


@pytest.mark.asyncio
@patch("asyncio.create_subprocess_exec")
async def test_start_node_failure(mock_exec, node_manager):
    node_id = "fail_node"
    # Provision but don't actually start
    await node_manager.provision_board(node_id, "machine:\n  cpus: []")

    # Mock process failure
    mock_proc = AsyncMock()
    mock_proc.returncode = 1
    mock_proc.stderr.read.return_value = b"Error message"
    mock_exec.return_value = mock_proc

    with pytest.raises(RuntimeError) as excinfo:
        await node_manager.start_node(node_id)

    assert "exited early with code 1" in str(excinfo.value)
    assert "Error message" in str(excinfo.value)
