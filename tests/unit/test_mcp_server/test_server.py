"""
SOTA Test Module: test_server

Context:
This module implements tests for the test_server subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_server.
"""

from unittest.mock import AsyncMock, MagicMock, patch

import mcp.types as types
import pytest
from mcp.types import TextContent

from tools.mcp_server.server import create_mcp_server


@pytest.fixture
def server():
    return create_mcp_server()


@pytest.mark.asyncio
async def test_list_tools(server):
    handler = server.request_handlers[types.ListToolsRequest]
    res = await handler(types.ListToolsRequest(method="tools/list"))
    tools_result = res.root
    tool_names = [t.name for t in tools_result.tools]
    assert "provision_board" in tool_names
    assert "start_node" in tool_names
    assert "read_cpu_state" in tool_names


@pytest.mark.asyncio
@patch("tools.mcp_server.server.NodeManager")
async def test_call_tool_provision(mock_manager_class):
    mock_manager = mock_manager_class.return_value
    mock_manager.provision_board = AsyncMock()

    with patch("tools.mcp_server.server.NodeManager", return_value=mock_manager):
        myserver = create_mcp_server()
        handler = myserver.request_handlers[types.CallToolRequest]

        req = types.CallToolRequest(
            method="tools/call",
            params=types.CallToolRequestParams(
                name="provision_board", arguments={"node_id": "n0", "board_config": "config", "config_type": "yaml"}
            ),
        )
        res = await handler(req)
        result = res.root.content  # type: ignore[union-attr]

        assert isinstance(result[0], TextContent)
        assert "Board provisioned" in result[0].text
        mock_manager.provision_board.assert_called_once_with("n0", "config", "yaml")


@pytest.mark.asyncio
async def test_call_tool_unknown(server):
    handler = server.request_handlers[types.CallToolRequest]
    req = types.CallToolRequest(
        method="tools/call", params=types.CallToolRequestParams(name="unknown_tool", arguments={})
    )
    res = await handler(req)
    result = res.root.content
    assert "Error: Unknown tool" in result[0].text


@pytest.mark.asyncio
async def test_call_tool_read_memory(server):
    node_id = "mem_node"
    node = server.node_manager.get_node(node_id)
    node.qmp_bridge.execute = AsyncMock(return_value={})

    # Mock tempfile to control the path
    with (
        patch("tempfile.mkstemp", return_value=(0, "/tmp/mock_mem")),
        patch("pathlib.Path.unlink"),
        patch(
            "pathlib.Path.open",
            MagicMock(
                side_effect=[
                    # First open in node_manager is for some other reason?
                    # No, server.py uses it.
                    MagicMock(__enter__=lambda s: MagicMock(read=lambda: b"\xde\xad\xbe\xef"))  # noqa: ARG005
                ]
            ),
        ),
    ):
        handler = server.request_handlers[types.CallToolRequest]
        req = types.CallToolRequest(
            method="tools/call",
            params=types.CallToolRequestParams(
                name="read_memory", arguments={"node_id": node_id, "address": 0x1000, "size": 4}
            ),
        )
        res = await handler(req)
        assert "deadbeef" in res.root.content[0].text


@pytest.mark.asyncio
async def test_call_tool_send_uart(server):
    node_id = "uart_node"
    node = server.node_manager.get_node(node_id)
    node.qmp_bridge.write_to_uart = AsyncMock()

    handler = server.request_handlers[types.CallToolRequest]
    req = types.CallToolRequest(
        method="tools/call",
        params=types.CallToolRequestParams(name="send_uart_input", arguments={"node_id": node_id, "data": "hello"}),
    )
    res = await handler(req)
    assert "Sent 5 bytes" in res.root.content[0].text
    node.qmp_bridge.write_to_uart.assert_called_once_with("hello")


@pytest.mark.asyncio
async def test_read_resource_console(server):
    node_id = "res_node"
    node = server.node_manager.get_node(node_id)
    node.qmp_bridge.uart_buffer = "console output"

    handler = server.request_handlers[types.ReadResourceRequest]
    req = types.ReadResourceRequest(
        method="resources/read",
        params=types.ReadResourceRequestParams(uri=f"virtmcu://nodes/{node_id}/console"),  # type: ignore[arg-type]
    )
    res = await handler(req)
    assert res.root.contents[0].text == "console output"
