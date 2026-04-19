import logging
import os

from mcp.server import Server
from mcp.types import (
    Resource,
    TextContent,
    Tool,
)

from tools.mcp_server.node_manager import NodeManager

logger = logging.getLogger(__name__)


def create_mcp_server() -> Server:
    server = Server("virtmcu-mcp")
    server.node_manager = NodeManager()

    @server.list_tools()
    async def handle_list_tools() -> list[Tool]:
        """List available tools for the AI agent."""
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
    async def handle_call_tool(name: str, arguments: dict) -> list[TextContent]:
        """Handle execution of tools."""
        try:
            if name == "provision_board":
                node_id = arguments["node_id"]
                await server.node_manager.provision_board(
                    node_id, arguments["board_config"], arguments.get("config_type", "yaml")
                )
                return [TextContent(type="text", text=f"Board provisioned for node {node_id}.")]

            elif name == "flash_firmware":
                node_id = arguments["node_id"]
                server.node_manager.flash_firmware(node_id, arguments["firmware_path"])
                return [
                    TextContent(
                        type="text", text=f"Firmware '{arguments['firmware_path']}' associated with node {node_id}."
                    )
                ]

            elif name == "start_node":
                node_id = arguments["node_id"]
                await server.node_manager.start_node(node_id)
                return [TextContent(type="text", text=f"Node {node_id} started.")]

            elif name == "stop_node":
                node_id = arguments["node_id"]
                await server.node_manager.stop_node(node_id)
                return [TextContent(type="text", text=f"Node {node_id} stopped.")]

            elif name == "pause_node":
                node = server.node_manager.get_node(arguments["node_id"])
                await node.qmp_bridge.pause_emulation()
                return [TextContent(type="text", text=f"Node {arguments['node_id']} paused.")]

            elif name == "resume_node":
                node = server.node_manager.get_node(arguments["node_id"])
                await node.qmp_bridge.start_emulation()
                return [TextContent(type="text", text=f"Node {arguments['node_id']} resumed.")]

            elif name == "read_cpu_state":
                node = server.node_manager.get_node(arguments["node_id"])
                hmp_res = await node.qmp_bridge.execute("human-monitor-command", {"command-line": "info registers"})
                return [TextContent(type="text", text=hmp_res)]

            elif name == "read_memory":
                node = server.node_manager.get_node(arguments["node_id"])
                addr = arguments["address"]
                size = arguments["size"]
                if size > 1024 * 1024:  # 1MB limit for safety
                     return [TextContent(type="text", text="Error: Memory read size too large (max 1MB)")]

                # pmemsave saves to a file, so we do it via QMP then read it
                import tempfile

                fd, tmp_path = tempfile.mkstemp()
                os.close(fd)
                try:
                    await node.qmp_bridge.execute("pmemsave", {"val": addr, "size": size, "filename": tmp_path})
                    with open(tmp_path, "rb") as f:
                        data = f.read()
                    hex_data = data.hex()
                    return [TextContent(type="text", text=f"Memory at {hex(addr)} ({size} bytes):\n{hex_data}")]
                finally:
                    os.remove(tmp_path)

            elif name == "disassemble":
                node = server.node_manager.get_node(arguments["node_id"])
                addr = arguments["address"]
                if addr == -1:
                    addr = await node.qmp_bridge.get_pc()
                count = arguments.get("instructions", 10)
                # No direct QMP disassemble, use HMP
                hmp_res = await node.qmp_bridge.execute(
                    "human-monitor-command", {"command-line": f"x/{count}i {hex(addr)}"}
                )
                return [TextContent(type="text", text=hmp_res)]

            elif name == "inject_interrupt":
                # For now we'll simulate via an HMP command if available, or return unsupported.
                return [
                    TextContent(
                        type="text",
                        text=f"inject_interrupt is not fully supported via QMP yet. Node {arguments['node_id']} IRQ {arguments['irq_number']}",
                    )
                ]

            elif name == "send_uart_input":
                node = server.node_manager.get_node(arguments["node_id"])
                await node.qmp_bridge.write_to_uart(arguments["data"])
                return [
                    TextContent(
                        type="text", text=f"Sent {len(arguments['data'])} bytes to UART of node {arguments['node_id']}."
                    )
                ]

            elif name == "set_network_latency":
                import json

                session = server.node_manager.get_zenoh_session()
                topic = "sim/network/control"
                data = {
                    "node_a": arguments["node_a"],
                    "node_b": arguments["node_b"],
                    "latency_ns": arguments["latency_ns"],
                }
                session.put(topic, json.dumps(data).encode("utf-8"))
                return [
                    TextContent(
                        type="text",
                        text=f"Published latency update for {arguments['node_a']}<->{arguments['node_b']}: {arguments['latency_ns']}ns.",
                    )
                ]

            else:
                raise ValueError(f"Unknown tool: {name}")
        except (Exception, BaseException) as e:
            logger.error(f"Error executing tool {name}: {e}")
            return [TextContent(type="text", text=f"Error: {str(e)}")]


    @server.list_resources()
    async def handle_list_resources() -> list[Resource]:
        """List available resources for the AI agent."""
        resources = [
            Resource(
                uri="virtmcu://simulation/status",
                name="Simulation Status",
                mimeType="application/json",
                description="A global view of all running nodes and their status.",
            )
        ]

        # Add UART console for each running node
        for node_id, node in server.node_manager.nodes.items():
            if node.process and node.process.returncode is None:
                resources.append(
                    Resource(
                        uri=f"virtmcu://nodes/{node_id}/console",
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
            status = {"status": "running", "nodes": []}
            for node_id, node in server.node_manager.nodes.items():
                node_status = "running" if (node.process and node.process.returncode is None) else "stopped"
                status["nodes"].append({"id": node_id, "status": node_status})
            import json

            return json.dumps(status)

        if uri.startswith("virtmcu://nodes/") and uri.endswith("/console"):
            parts = uri.split("/")
            node_id = parts[3]
            if node_id in server.node_manager.nodes:
                node = server.node_manager.nodes[node_id]
                return node.qmp_bridge.uart_buffer
            raise ValueError(f"Node {node_id} not found")

        raise ValueError(f"Unknown resource URI: {uri}")

    return server
