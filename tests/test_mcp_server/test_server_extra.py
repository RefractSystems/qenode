import json
from unittest.mock import AsyncMock, MagicMock, patch

import mcp.types as types
import pytest

from tools.mcp_server.server import create_mcp_server


@pytest.fixture
def server():
    return create_mcp_server()


@pytest.mark.asyncio
async def test_call_tool_pause_resume(server):
    node_id = "p_r_node"
    node = server.node_manager.get_node(node_id)
    node.qmp_bridge.pause_emulation = AsyncMock()
    node.qmp_bridge.start_emulation = AsyncMock()

    handler = server.request_handlers[types.CallToolRequest]

    # Pause
    req = types.CallToolRequest(
        method="tools/call", params=types.CallToolRequestParams(name="pause_node", arguments={"node_id": node_id})
    )
    await handler(req)
    node.qmp_bridge.pause_emulation.assert_called_once()

    # Resume
    req = types.CallToolRequest(
        method="tools/call", params=types.CallToolRequestParams(name="resume_node", arguments={"node_id": node_id})
    )
    await handler(req)
    node.qmp_bridge.start_emulation.assert_called_once()


@pytest.mark.asyncio
async def test_call_tool_disassemble(server):
    node_id = "dis_node"
    node = server.node_manager.get_node(node_id)
    node.qmp_bridge.execute = AsyncMock(return_value="0x1000: mov r0, #0")
    node.qmp_bridge.get_pc = AsyncMock(return_value=0x1000)

    handler = server.request_handlers[types.CallToolRequest]
    req = types.CallToolRequest(
        method="tools/call",
        params=types.CallToolRequestParams(name="disassemble", arguments={"node_id": node_id, "address": -1}),
    )
    res = await handler(req)
    assert "mov r0" in res.root.content[0].text
    node.qmp_bridge.get_pc.assert_called_once()


@pytest.mark.asyncio
@patch("zenoh.open")
async def test_call_tool_set_network_latency(mock_zenoh_open, server):
    mock_session = MagicMock()
    mock_zenoh_open.return_value = mock_session

    # We need to mock node_manager.get_zenoh_session since it's already defined
    server.node_manager._zenoh_session = mock_session

    handler = server.request_handlers[types.CallToolRequest]
    req = types.CallToolRequest(
        method="tools/call",
        params=types.CallToolRequestParams(
            name="set_network_latency", arguments={"node_a": "n0", "node_b": "n1", "latency_ns": 1000}
        ),
    )
    await handler(req)
    mock_session.put.assert_called_once()
    topic, payload = mock_session.put.call_args[0]
    assert topic == "sim/network/control"
    assert b"1000" in payload


@pytest.mark.asyncio
async def test_list_resources(server):
    node_id = "node_res"
    node = server.node_manager.get_node(node_id)
    node.process = MagicMock()
    node.process.returncode = None

    handler = server.request_handlers[types.ListResourcesRequest]
    res = await handler(types.ListResourcesRequest(method="resources/list"))
    resources = res.root.resources
    uris = [str(r.uri) for r in resources]
    assert "virtmcu://simulation/status" in uris
    assert f"virtmcu://nodes/{node_id}/console" in uris


@pytest.mark.asyncio
async def test_read_resource_status(server):
    node_id = "node_status"
    node = server.node_manager.get_node(node_id)
    node.process = MagicMock()
    node.process.returncode = None

    handler = server.request_handlers[types.ReadResourceRequest]
    req = types.ReadResourceRequest(
        method="resources/read", params=types.ReadResourceRequestParams(uri="virtmcu://simulation/status")
    )
    res = await handler(req)
    status = json.loads(res.root.contents[0].text)
    assert status["status"] == "running"
    assert any(n["id"] == node_id and n["status"] == "running" for n in status["nodes"])


@pytest.mark.asyncio
async def test_call_tool_flash_firmware(server):
    node_id = "flash_node"
    with patch("os.path.exists", return_value=True):
        with patch("os.path.isabs", return_value=True):
            handler = server.request_handlers[types.CallToolRequest]
            req = types.CallToolRequest(
                method="tools/call",
                params=types.CallToolRequestParams(
                    name="flash_firmware", arguments={"node_id": node_id, "firmware_path": "/tmp/test.elf"}
                ),
            )
            res = await handler(req)
            assert "associated with node" in res.root.content[0].text
            assert server.node_manager.get_node(node_id).firmware_path == "/tmp/test.elf"


@pytest.mark.asyncio
async def test_call_tool_read_cpu_state(server):
    node_id = "cpu_node"
    node = server.node_manager.get_node(node_id)
    node.qmp_bridge.execute = AsyncMock(return_value="R0=00000000 R1=00000000")

    handler = server.request_handlers[types.CallToolRequest]
    req = types.CallToolRequest(
        method="tools/call", params=types.CallToolRequestParams(name="read_cpu_state", arguments={"node_id": node_id})
    )
    res = await handler(req)
    assert "R0=00000000" in res.root.content[0].text
