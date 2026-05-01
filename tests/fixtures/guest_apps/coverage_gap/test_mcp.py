"""
SOTA Test Module: test_mcp

Context:
This module implements tests for the test_mcp subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_mcp.
"""

import os

import pytest
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client


@pytest.mark.asyncio
async def test_mcp_server_lifecycle():
    """
    Verifies that the virtmcu-mcp server can start and respond to basic tools.
    """
    # 1. Start the MCP server as a subprocess
    server_params = StdioServerParameters(command="virtmcu-mcp", args=[], env=os.environ.copy())

    async with stdio_client(server_params) as (read, write), ClientSession(read, write) as session:
        # Initialize
        await session.initialize()

        # List tools
        tools = await session.list_tools()
        tool_names = [t.name for t in tools.tools]

        assert "provision_board" in tool_names
        assert "start_node" in tool_names
        assert "stop_node" in tool_names
        assert "read_memory" in tool_names

        # Test list_resources
        resources = await session.list_resources()
        # Initial list might be empty if no nodes are running
        assert resources is not None


@pytest.mark.asyncio
async def test_mcp_provision_and_fail():
    """
    Tests provisioning with a non-existent YAML file.
    """
    server_params = StdioServerParameters(command="virtmcu-mcp", args=[], env=os.environ.copy())

    async with stdio_client(server_params) as (read, write), ClientSession(read, write) as session:
        await session.initialize()

        # Attempt to provision a non-existent board
        # Note: board_config expects the CONTENT of the config, not the path.
        # But wait, if it's the content, let's see how it's handled.
        result = await session.call_tool(
            "provision_board",
            arguments={"node_id": "test_board", "board_config": "invalid content", "config_type": "yaml"},
        )

        # MCP tools return a list of content items
        assert result.isError is True
        assert "error" in result.content[0].text.lower()
