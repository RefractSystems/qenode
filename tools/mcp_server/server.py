"""List available tools for the AI agent."""

from __future__ import annotations

import logging
import os
from pathlib import Path
from typing import Any, cast

from mcp.server import Server
from mcp.types import (
    Resource,
    TextContent,
    Tool,
)
from pydantic import AnyUrl

from tools.mcp_server.node_manager import NodeManager

logger = logging.getLogger(__name__)



class VirtmcuServer(Server):  # type: ignore[misc]
    node_manager: NodeManager

def create_mcp_server() -> VirtmcuServer:
    server = VirtmcuServer("virtmcu-mcp")

    node_manager = NodeManager()
    server.node_manager = node_manager

    @server.list_tools()
    async def handle_list_tools() -> list[Tool]:

        return [
            Tool(
                name="provision_board",
                description="Accepts a YAML/REPL description, validates it, and prepares the simulation environment.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "The name/ID of the node (e.g. 'node0')"},
                        "board_config": {
                            "type": "string",
                            "description": "YAML or REPL configuration content for the board.",
                        },
                        "config_type": {"type": "string", "enum": ["yaml", "repl"], "default": "yaml"},
                    },
                    "required": ["node_id", "board_config"],
                },
            ),
            Tool(
                name="flash_firmware",
                description="Associates a firmware ELF or binary with a specific node.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node to flash."},
                        "firmware_path": {
                            "type": "string",
                            "description": "Absolute or workspace-relative path to the firmware file (.elf, .bin, .hex).",
                        },
                    },
                    "required": ["node_id", "firmware_path"],
                },
            ),
            Tool(
                name="start_node",
                description="Launches the QEMU instance for a node.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node to start."},
                    },
                    "required": ["node_id"],
                },
            ),
            Tool(
                name="stop_node",
                description="Terminally kills the QEMU process for a node.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node to stop."},
                    },
                    "required": ["node_id"],
                },
            ),
            Tool(
                name="pause_node",
                description="Uses QMP `stop` command to freeze execution of a node.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node to pause."},
                    },
                    "required": ["node_id"],
                },
            ),
            Tool(
                name="resume_node",
                description="Uses QMP `cont` command to resume execution of a node.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node to resume."},
                    },
                    "required": ["node_id"],
                },
            ),
            Tool(
                name="read_cpu_state",
                description="Returns registers (PC, SP, R0-R12) and current execution mode.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node."},
                    },
                    "required": ["node_id"],
                },
            ),
            Tool(
                name="read_memory",
                description="Dumps raw memory. Useful for inspecting task stacks or peripheral registers.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node."},
                        "address": {"type": "integer", "description": "Memory address to read from."},
                        "size": {"type": "integer", "description": "Number of bytes to read."},
                    },
                    "required": ["node_id", "address", "size"],
                },
            ),
            Tool(
                name="disassemble",
                description="Uses QMP to return a disassembly of the current or target code area.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node."},
                        "address": {
                            "type": "integer",
                            "description": "Memory address to disassemble from (use -1 for current PC).",
                        },
                        "instructions": {
                            "type": "integer",
                            "description": "Number of instructions to disassemble (default 10).",
                        },
                    },
                    "required": ["node_id", "address"],
                },
            ),
            Tool(
                name="inject_interrupt",
                description="Manually triggers a hardware interrupt for testing fault handlers.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node."},
                        "irq_number": {"type": "integer", "description": "IRQ number to trigger."},
                    },
                    "required": ["node_id", "irq_number"],
                },
            ),
            Tool(
                name="send_uart_input",
                description="Publishes bytes to the node's Zenoh UART RX topic or directly to socket if standalone.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string", "description": "ID of the node."},
                        "data": {"type": "string", "description": "Data to send to the UART."},
                    },
                    "required": ["node_id", "data"],
                },
            ),
            Tool(
                name="set_network_latency",
                description="Communicates with the zenoh_coordinator to manipulate the simulated RF environment.",
                inputSchema={
                    "type": "object",
                    "properties": {
                        "node_a": {"type": "string", "description": "ID of the first node."},
                        "node_b": {"type": "string", "description": "ID of the second node."},
                        "latency_ns": {"type": "integer", "description": "Latency in nanoseconds."},
                    },
                    "required": ["node_a", "node_b", "latency_ns"],
                },
            ),
        ]

    @server.call_tool()
    async def handle_call_tool(name: str, arguments: dict[str, object] | None) -> list[TextContent]:
        """Handle execution of tools."""
        if arguments is None:
            arguments = {}
        try:
            if name == "provision_board":
                node_id = cast(str, arguments["node_id"])
                await node_manager.provision_board(
                    node_id, cast(str, arguments["board_config"]), cast(str, arguments.get("config_type", "yaml"))
                )
                return [TextContent(type="text", text=f"Board provisioned for node {node_id}.")]

            if name == "flash_firmware":
                node_id = cast(str, arguments["node_id"])
                node_manager.flash_firmware(node_id, cast(str, arguments["firmware_path"]))
                return [
                    TextContent(
                        type="text", text=f"Firmware '{arguments['firmware_path']}' associated with node {node_id}."
                    )
                ]

            if name == "start_node":
                node_id = cast(str, arguments["node_id"])
                await node_manager.start_node(node_id)
                return [TextContent(type="text", text=f"Node {node_id} started.")]

            if name == "stop_node":
                node_id = cast(str, arguments["node_id"])
                await node_manager.stop_node(node_id)
                return [TextContent(type="text", text=f"Node {node_id} stopped.")]

            if name == "pause_node":
                node = node_manager.get_node(cast(str, arguments["node_id"]))
                await node.qmp_bridge.pause_emulation()
                return [TextContent(type="text", text=f"Node {arguments['node_id']} paused.")]

            if name == "resume_node":
                node = node_manager.get_node(cast(str, arguments["node_id"]))
                await node.qmp_bridge.start_emulation()
                return [TextContent(type="text", text=f"Node {arguments['node_id']} resumed.")]

            if name == "read_cpu_state":
                node = node_manager.get_node(cast(str, arguments["node_id"]))
                hmp_res = await node.qmp_bridge.execute("human-monitor-command", {"command-line": "info registers"})
                return [TextContent(type="text", text=cast(str, hmp_res))]

            if name == "read_memory":
                node = node_manager.get_node(cast(str, arguments["node_id"]))
                addr = cast(int, arguments["address"])
                size = cast(int, arguments["size"])
                if size > 1024 * 1024:  # 1MB limit for safety
                    return [TextContent(type="text", text="Error: Memory read size too large (max 1MB)")]

                # pmemsave saves to a file, so we do it via QMP then read it
                import tempfile

                fd, tmp_path = tempfile.mkstemp()
                os.close(fd)
                try:
                    await node.qmp_bridge.execute("pmemsave", {"val": addr, "size": size, "filename": tmp_path})
                    with Path(tmp_path).open("rb") as f:
                        data = f.read()
                    hex_data = data.hex()
                    return [TextContent(type="text", text=f"Memory at {hex(addr)} ({size} bytes):\n{hex_data}")]
                finally:
                    Path(tmp_path).unlink()

            elif name == "disassemble":
                node = node_manager.get_node(cast(str, arguments["node_id"]))
                addr = cast(int, arguments["address"])
                if addr == -1:
                    addr = await node.qmp_bridge.get_pc()
                count = cast(int, arguments.get("instructions", 10))
                # No direct QMP disassemble, use HMP
                hmp_res = await node.qmp_bridge.execute(
                    "human-monitor-command", {"command-line": f"x/{count}i {hex(addr)}"}
                )
                return [TextContent(type="text", text=cast(str, hmp_res))]

            elif name == "inject_interrupt":
                # For now we'll simulate via an HMP command if available, or return unsupported.
                return [
                    TextContent(
                        type="text",
                        text=f"inject_interrupt is not fully supported via QMP yet. Node {arguments['node_id']} IRQ {arguments['irq_number']}",
                    )
                ]

            elif name == "send_uart_input":
                node = node_manager.get_node(cast(str, arguments["node_id"]))
                await node.qmp_bridge.write_to_uart(cast(str, arguments["data"]))
                return [
                    TextContent(
                        type="text",
                        text=f"Sent {len(cast(str, arguments['data']))} bytes to UART of node {arguments['node_id']}.",
                    )
                ]

            elif name == "set_network_latency":
                import json

                session = node_manager.get_zenoh_session()
                topic = "sim/network/control"
                latency_data = {
                    "node_a": arguments["node_a"],
                    "node_b": arguments["node_b"],
                    "latency_ns": arguments["latency_ns"],
                }
                session.put(topic, json.dumps(latency_data).encode("utf-8"))
                return [
                    TextContent(
                        type="text",
                        text=f"Published latency update for {arguments['node_a']}<->{arguments['node_b']}: {arguments['latency_ns']}ns.",
                    )
                ]

            else:
                raise ValueError(f"Unknown tool: {name}")
        except Exception as e:  # noqa: BLE001
            # Prevent the whole server from crashing if a tool execution fails
            logger.error(f"Error executing tool {name}: {e}")
            return [TextContent(type="text", text=f"Error: {e!s}")]

    @server.list_resources()
    async def handle_list_resources() -> list[Resource]:
        """List available resources for the AI agent."""
        resources = [
            Resource(
                uri=AnyUrl("virtmcu://simulation/status"),
                name="Simulation Status",
                mimeType="application/json",
                description="A global view of all running nodes and their status.",
            )
        ]

        # Add UART console for each running node
        for node_id, node in node_manager.nodes.items():
            if node.process and node.process.returncode is None:
                resources.append(
                    Resource(
                        uri=AnyUrl(f"virtmcu://nodes/{node_id}/console"),
                        name=f"Console - {node_id}",
                        mimeType="text/plain",
                        description=f"Real-time UART output stream for node {node_id}.",
                    )
                )

        return resources

    @server.read_resource()
    async def handle_read_resource(uri: str) -> str:
        uri = str(uri)
        if uri == "virtmcu://simulation/status":
            status: dict[str, Any] = {"status": "running", "nodes": []}
            for node_id, node in node_manager.nodes.items():
                node_status = "running" if (node.process and node.process.returncode is None) else "stopped"
                status["nodes"].append({"id": node_id, "status": node_status})
            import json

            return json.dumps(status)

        if uri.startswith("virtmcu://nodes/") and uri.endswith("/console"):
            parts = uri.split("/")
            node_id = parts[3]
            if node_id in node_manager.nodes:
                node = node_manager.nodes[node_id]
                assert node.qmp_bridge is not None
                return node.qmp_bridge.uart_buffer
            raise ValueError(f"Node {node_id} not found")

        raise ValueError(f"Unknown resource URI: {uri}")

    return server
