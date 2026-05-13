# Transport Layer

## The Physical Foundation

As introduced in the System Overview, the **Transport Layer** is the critical interconnect bridging the **Cyber Node** (e.g., QEMU) and the **Physical Node** (e.g., MuJoCo), as well as routing traffic between multiple Cyber nodes. 

The VirtMCU transport layer manages the physical movement of bytes between these simulation components. It provides a unified interface that abstracts the complexities of network sockets and inter-process communication, ensuring that the theoretical requirements of a Cyber-Physical System (CPS) are met in practice.

---

## 1. The Dual-Transport Architecture

While conceptually any protocol could serve as the Transport Layer, VirtMCU currently implements two primary mechanisms to balance performance and scalability:

### Unix Domain Sockets (LPSC)
*   **Best for**: Single-host co-simulation and local integration tests.
*   **Performance**: Extremely low latency (1–3 µs RTT).
*   **Usage**: Default for `ClockSyncTransport` and the `mmio-socket-bridge`.

### Eclipse Zenoh (Federated)
*   **Best for**: Distributed multi-node simulation across containers or clusters.
*   **Performance**: High throughput with moderate latency (10–50 µs RTT).
*   **Discovery**: Managed via explicit TCP endpoints to prevent cross-CI multicast collisions.

---

## 2. Connectivity & Discovery Guardrails

One of the most significant challenges in distributed simulation is ensuring that all nodes are connected before the first instruction executes. VirtMCU implements two key guardrails to prevent "First Message Loss":

### Liveliness Blocking
When a VirtMCU node starts, the `transport-zenoh` layer blocks initialization until it receives a **Liveliness Event** from the Zenoh router. This ensures that the local router is reachable and its topology tables are ready to accept traffic.

### Orchestrator Sequencing
The simulation orchestrator (e.g., Python `VirtualPhysicalNode`) is responsible for establishing all subscribers **before** launching the emulator nodes. This "Subscriber-First" policy ensures that when a guest peripheral emits its first packet, the routing fabric is already primed to deliver it.

### Routing Synchronization (`ensure_session_routing`)
Declaring a subscriber in Zenoh is an asynchronous operation. To prevent races where the first virtual-time packet is dropped because the router has not yet fully propagated the declaration, VirtMCU provides the `ensure_session_routing(session)` helper.

**Automation**: This synchronization is handled automatically by the `Simulation` framework and the `coordinator_subprocess` context manager. It uses a liveliness-probe roundtrip to verify that the router has fully ingested the session's declaration backlog before emulated code begins to execute. Tests MUST NOT call this helper manually.

---

## 3. Resilience and Fail-Fast Behavior

VirtMCU assumes a reliable underlying transport. If a Zenoh router or Unix socket disconnects during a simulation:
- **Immediate Failure**: The node will typically panic or report a catastrophic `TRANSPORT_ERROR`.
- **No Silent Drops**: We prioritize stopping the simulation over continuing with potentially corrupt or missing data, ensuring that every result is either perfectly deterministic or explicitly failed.

---

## See Also
*   **[Communication Protocols](./04-communication-protocols.md)**: The logical layer built upon this transport foundation.
*   **[PDES and Virtual Time](../fundamentals/08-pdes-and-virtual-time.md)**: How transport latency affects simulation performance.
stic or explicitly failed.
