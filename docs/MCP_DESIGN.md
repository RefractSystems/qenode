# Design Document: virtmcu MCP Server

## 1. Goal
Provide a Model Context Protocol (MCP) interface that allows AI agents to architect, deploy, and debug bare-metal firmware simulations. The MCP server acts as an out-of-band "AI Co-Pilot" for the `virtmcu` engine, abstracting low-level QMP and Zenoh protocols into high-level semantic tools.

## 2. Architecture
The MCP server is a standalone Python process located in `tools/mcp_server/`. 

- **Northbound**: Speaks MCP (over stdio or SSE) to AI clients (Claude Desktop, Gemini CLI, etc.).
- **Southbound (Control)**: Connects to QEMU nodes via **QMP** (Unix Sockets) for CPU/Memory control.
- **Southbound (Data)**: Connects to the **Zenoh Federation Bus** for interactive I/O (UART, Network, Sensors).
- **Toolchain Integration**: Invokes `tools/yaml2qemu.py` and `scripts/run.sh` to manage the simulation lifecycle.

---

## 3. Tool Definitions (Actions)

### 3.1 Lifecycle Management
- **`provision_board(board_config: str, name: str)`**: Accepts a YAML/REPL description. Validates it via `yaml2qemu` and prepares the simulation environment.
- **`flash_firmware(node_id: str, firmware_path: str)`**: Uploads an ELF or binary to the workspace and associates it with a specific node.
- **`start_node(node_id: str)`**: Launches the QEMU instance. If in a slaved mode (Phase 7), the node will wait for a TimeAuthority clock advance.
- **`stop_node(node_id: str)`**: Terminally kills the QEMU process.
- **`pause_node(node_id: str)`**: Uses QMP `stop` command to freeze execution.
- **`resume_node(node_id: str)`**: Uses QMP `cont` command to resume execution.

### 3.2 Debugging & Inspection
- **`read_cpu_state(node_id: str)`**: Returns registers (PC, SP, R0-R12) and current execution mode.
- **`read_memory(node_id: str, address: int, size: int)`**: Dumps raw memory. Useful for inspecting task stacks or peripheral registers.
- **`disassemble(node_id: str, address: int, instructions: int)`**: Uses QMP to return a disassembly of the current or target code area.
- **`inject_interrupt(node_id: str, irq_number: int)`**: Manually triggers a hardware interrupt for testing fault handlers.

### 3.3 Interactive I/O
- **`send_uart_input(node_id: str, data: str)`**: Publishes bytes to the node's Zenoh UART RX topic.
- **`set_network_latency(node_a: str, node_b: str, latency_ns: int)`**: Communicates with the `zenoh_coordinator` to manipulate the simulated RF environment.

---

## 4. Resource Definitions (Observability)

- **`virtmcu://nodes/{node_id}/console`**: A real-time stream (or tail) of the node's UART output.
- **`virtmcu://nodes/{node_id}/hardware_map`**: The JSON-serialized representation of the board layout (peripherals, base addresses, IRQ lines).
- **`virtmcu://simulation/status`**: A global view of all running nodes, their virtual clock time, and CPU utilization.

---

## 5. Security & Safety
- The MCP server will only access files within the `virtmcu` workspace and `/tmp/virtmcu`.
- QMP sockets must be local Unix sockets (not TCP) to prevent unauthorized remote control of the host.
- Firmware uploads are restricted to `.elf`, `.bin`, and `.hex` formats.

---

## 6. Implementation Strategy (Phase 13)
1. Implement the MCP server base using the `mcp` SDK.
2. Wrap `tools/testing/qmp_bridge.py` for all QMP logic.
3. Use `eclipse-zenoh` Python bindings for UART/Network interaction.
4. Add lesson 13 to the tutorial: "AI-Augmented Firmware Debugging with MCP."

---

## 7. Commands
- **Run the Server**: `python3 -m tools.mcp_server` (will communicate over stdio by default).
- **Run the Tests**: `pytest tests/mcp_server/`
- **Lint the Code**: `make lint` and `make fmt` (uses `ruff`).

## 8. Code Style
The Python code will follow the established `virtmcu` standards enforced by `ruff`.
```python
# Example: Use type hints, concise logging, and async where appropriate
import logging
from mcp.server import Server
from tools.testing.qmp_bridge import QmpBridge

logger = logging.getLogger(__name__)

async def read_cpu_state(node_id: str) -> dict:
    """Returns the CPU registers and state via QMP."""
    bridge = QmpBridge(f"/tmp/virtmcu/{node_id}.qmp")
    await bridge.connect()
    # Logic to fetch CPU state
    return {"pc": 0x08000000}
```

## 9. Testing Strategy
- **Unit Tests (`tests/mcp_server/`)**: Mock QMP sockets and Zenoh connections to verify MCP tool dispatching, payload validation, and JSON serialization.
- **Integration Tests**: Extend `test/phase13/smoke_test.sh` to spin up a mock AI client that invokes `provision_board` and `read_cpu_state` on a real QEMU instance.

## 10. Boundaries
- **Always do**: Validate all inputs from the MCP client to prevent injection attacks (e.g., path traversal when flashing firmware). Use `asyncio` for all QMP/Zenoh I/O so the MCP server stays responsive.
- **Ask first**: Before adding large binary dependencies or changing the underlying QMP bridge protocol.
- **Never do**: Expose TCP QMP sockets to the host network. Never run the MCP server as `root`.
