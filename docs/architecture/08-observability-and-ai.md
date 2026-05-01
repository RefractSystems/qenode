# Chapter 8: Observability & AI Co-pilot

## Seeing the Unseen

As VirtMCU evolves from a foundational emulator into a robust digital twin environment, observability and AI accessibility become first-class concerns. We provide deep introspection without embedding complex GUIs into the emulator core.

---

## 1. High-Fidelity Telemetry

VirtMCU provides rich, interactive observability into the guest's execution. By tracing CPU sleep states, peripheral events, and register mutations, we publish deterministic timelines over the simulation bus.

### Event Streaming
The `hw/rust/observability/telemetry` layer publishes low-overhead FlatBuffer events:
- **CPU State**: Tracking when a node enters/exits `WFI` (Wait For Interrupt).
- **IRQ Tracing**: Recording exactly when and why an interrupt line was asserted.
- **Peripheral Events**: High-level semantic logs (e.g., "UART FIFO Full", "CAN ID Matched").

These streams can be ingested by visual timeline tools or analyzed by the test harness to verify complex timing requirements.

---

## 2. The MCP Co-pilot Interface

To support LLM-driven debugging and lifecycle management, VirtMCU includes a standalone **Model Context Protocol (MCP)** server (`tools/mcp_server/`). This interface allows AI agents to act as "peer programmers" in the simulation environment.

### Capabilities for AI Agents:
- **Control**: AI agents can provision boards, flash firmware, and control node lifecycle (start/stop/pause).
- **Introspection**: Agents can inspect raw memory, read CPU registers, and disassemble guest code dynamically via the `qmp_bridge.py` wrapper.
- **Interactive Debugging**: Agents can interact with UART consoles, monitor network traffic, and inject faults to verify firmware resilience.

---

## 3. Semantic Debugging

Because VirtMCU is deterministic, we can perform **Record & Replay** debugging. 
1.  **Record**: Run a simulation and log all telemetry and network traffic to a PCAP or JSON oracle.
2.  **Analyze**: An AI agent or human engineer analyzes the trace to identify the exact virtual nanosecond where a bug occurred.
3.  **Replay**: Re-run the simulation with the same seed and a GDB debugger attached. The bug will manifest at the exact same point, every time.

This removes the "Heisenbug" problem from embedded software development, making even the most complex multi-node races reliably reproducible.

### PCAP Link-Layer Schema
VirtMCU exports binary network and telemetry traces using the **DLT_USER0 (147)** link layer. We multiplex the different node protocols via a 2-byte protocol identifier immediately following the standard 8-byte src/dst routing header:
*   Protocol `1`: Ethernet
*   Protocol `2`: UART
*   Protocol `3`: IEEE 802.15.4
*   Protocol `4`: CAN-FD
*   Protocol `5`: FlexRay
*   Protocol `255`: VirtMCU Test Infrastructure (Python topics, direction markers).
