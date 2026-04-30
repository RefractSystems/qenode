# Chapter 4: Communication Protocols

## The Logical Plane

While the Transport Layer (Chapter 3) moves raw bytes, the Communication Protocol layer defines the **meaning** of those bytes. VirtMCU standardizes all logical interactions on a deterministic, schema-first Data Plane.

---

## 1. The Data Plane (Simulation Bus)

The Data Plane is the virtual "backplane" where all emulated inter-node traffic flows. Every Ethernet frame, UART byte, and CAN-FD message is treated as a discrete event on this bus.

### Causal Ordering & Sorting
The Data Plane is fundamentally different from a standard network. Within the simulation, all traffic is governed by the **Deterministic Coordinator**. 
1.  **Buffering**: All nodes push messages to the coordinator during a quantum.
2.  **Canonical Sorting**: The coordinator sorts all messages by `(vtime_ns, src_id, seq_num)`.
3.  **Delivery**: Messages are only delivered once the coordinator confirms the entire quantum is "frozen."

---

## 2. Serialization: FlatBuffers (`core.fbs`)

To ensure binary fidelity across C, Rust, and Python, VirtMCU uses **FlatBuffers**. Unlike JSON or Protobuf, FlatBuffers allows for zero-copy access to data, which is critical for maintaining high-frequency simulation throughput.

### The Unified IDL
Every packet on the simulation bus follows the definitions in `hw/rust/common/virtmcu-api/src/core.fbs`:
-   **Headers**: All emulated network traffic (Ethernet, UART, RF) is prefixed with a `ZenohFrameHeader` (24 bytes). This header encapsulates the `delivery_vtime_ns` and sequence numbers required for deterministic sorting.
-   **Payloads**: High-speed signals like `MmioReq` and `ClockAdvanceReq` use strictly aligned FlatBuffer structs to ensure bit-identical layout on all architectures.

---

## 3. The Topic Map (Addressing Scheme)

VirtMCU uses a hierarchical topic map to route messages across the simulation bus. When operating over Zenoh, these translate directly into KeyExpressions.

**Note:** `{node_id}` is a unique integer or string assigned to each simulated cyber node or physics entity.

### Synchronous Control Channels
The Control Plane uses synchronous requests (e.g., Zenoh Queryables) to ensure the emulator pauses while waiting for a response.
*   **Clock Sync**: `sim/clock/advance/{node_id}`

### Deterministic Sub-systems (Asynchronous)
All cyber-world network traffic is published asynchronously and routed through the Deterministic Coordinator.
*   **Ethernet**: `sim/eth/frame/{node_id}/tx` → `sim/eth/frame/{node_id}/rx`
*   **UART (Serial)**: `virtmcu/uart/{node_id}/tx` → `virtmcu/uart/{node_id}/rx`
*   **802.15.4 Radio**: `sim/rf/ieee802154/{node_id}/tx` → `sim/rf/ieee802154/{node_id}/rx`
*   **Bluetooth HCI**: `sim/rf/hci/{node_id}/tx` → `sim/rf/hci/{node_id}/rx`

### Cyber-Physical & Observability
These topics bridge the cyber world with the physics engine and external observability tools.
*   **Sensors**: `sim/sensor/{node_id}/{name}` (Physics data ingress)
*   **Actuators**: `sim/actuator/{node_id}/{name}` (Control data egress)
*   **UI LEDs**: `sim/ui/{node_id}/led/{led_id}`
*   **UI Buttons**: `sim/ui/{node_id}/button/{btn_id}`
*   **Telemetry**: `sim/telemetry/trace/{node_id}` (High-fidelity CPU and IRQ traces)
