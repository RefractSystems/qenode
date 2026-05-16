# RFC-0019: Native IPC Hybrid Architecture for Single-Host Co-Simulation

## Status
Accepted

## Context
VirtMCU relies on Eclipse Zenoh as its primary transport layer (RFC-0011). While Zenoh excels in distributed, multi-node environments, running complex cyber-physical simulations (e.g., QEMU + MuJoCo + NextJS UI) strictly on a **single host machine** introduces unnecessary overhead and race conditions. 

Zenoh is an asynchronous Pub/Sub broker. When used locally, this decoupling leads to orchestration headaches: ensuring subscribers are ready before publishers broadcast, managing non-deterministic broker routing latency, and building manual synchronization barriers on top of an inherently asynchronous framework.

To achieve State-of-the-Art (SOTA) performance and absolute determinism for single-host execution, we must bypass the network broker entirely and leverage native operating system Inter-Process Communication (IPC).

## Decision
For single-host deployments, VirtMCU will utilize a **Hybrid Native IPC Architecture** consisting of **Unix Domain Sockets (UDS)** for all event/control routing, and **Shared Memory (SHM) + Linux Futex** strictly for physical state synchronization. Zenoh will be entirely bypassed in this mode.

To achieve State-of-the-Art (SOTA) performance and minimize memory allocations on the hot path, the UDS event transport implements the **Transport-Agnostic Reservation API** defined in **RFC-0025 (canonical)**. RFC-0025 owns the trait/API surface; this RFC owns only the UDS *backend* — thread-local arenas, kernel framing, and lifecycle (`EOF`-on-crash semantics).

The on-the-wire framing, registration handshake (`UdsRegistration`), and quantum-start signalling (`UdsQuantumStart`) are specified in **RFC-0033 (UDS Coordinator Wire Protocol)**. FlatBuffers schemas live in `hw/rust/common/virtmcu-api/src/core.fbs`.

### The Single-Host Hybrid Architecture

1. **Physics State (MuJoCo ↔ Gateway): SHM + Futex**
   - **Mechanism:** A single shared memory file (`/dev/shm/virtmcu_physics_0`) holds arrays of physical attributes (Sensors and Actuators).
   - **Synchronization:** Lock-step execution is enforced using a two-phase doorbell backed by the Linux `futex(2)` system call. The Physics Engine and Gateway sleep at 0% CPU and are woken instantly by the kernel when the sequence counters (`bridge_seq` and `physics_seq`) change.

2. **Event & Control Plane (QEMU ↔ Time Authority ↔ Coordinator): UDS + Thread-Local Arenas**
   - **Mechanism:** Direct, point-to-point Unix Domain Socket connections.
   - **Zero-Allocation API:** Peripherals use a `reserve()`/`commit()` API (RFC-0025) backed by thread-local arenas. `reserve()` provides a lock-free, zero-allocation mutable slice. `commit()` performs a single `write()` system call to push the arena buffer down the UDS socket.
   - **Data Plane (Networking):** Instead of publishing Ethernet/UART frames to Zenoh, QEMU nodes write them to a UDS connected to the `DeterministicCoordinator`. The Coordinator buffers these frames.
   - **PDES Barrier:** At the end of a quantum, QEMU nodes send a `CoordDoneReq` over UDS. The Coordinator waits for all sockets to report done, sorts the buffered network frames by `vtime_ns`, and pushes them down the destination UDS pipes.

3. **User Interface (NextJS Frontend): WebSockets / Observability**
   - **Mechanism:** The UI connects as a read-only observer to the simulation's telemetry streams (via WebSockets or by querying the OpenTelemetry/Loki/Tempo stack).
   - **Real-Time Monitoring:** Real-time state (CPU WFI transitions, sensor data, IRQ assertions) is driven strictly by the low-overhead FlatBuffer telemetry emitted by the TCG Tracer and Physics Gateway.
   - **Interactive Debugging (Optional):** If the UI requires "Debugger" features (e.g., pause, step, read arbitrary memory), it bridges to QEMU's JSON-RPC Machine Protocol (QMP) over a UDS socket. However, QMP is strictly for control and deep introspection, **never** for polling continuous state.

## Architecture Comparison

The following diagrams illustrate the data flow and transport mechanics for a multi-node simulation (2x QEMU + 1x MuJoCo) under both the legacy Zenoh and the new Native IPC architectures.

### Option A: Distributed Architecture (Legacy / Zenoh)

```mermaid
graph TD
    subgraph "Host / Cluster"
        Z[Zenoh Router]
        
        Q0["QEMU 0 (VirtMCU)"]
        Q1["QEMU 1 (VirtMCU)"]
        TA["Time Authority"]
        DC["Deterministic Coordinator"]
        PG["Physics Gateway"]
        PE["MuJoCo (Physics Engine)"]
        UI["NextJS UI"]
        OBS["Observability / OTel"]

        Q0 <-->|TCP/SHM: pub/sub (ZenohFrameHeader)<br/>firmware/control/**| Z
        Q1 <-->|TCP/SHM: pub/sub (ZenohFrameHeader)<br/>firmware/control/**| Z
        
        Z <-->|TCP/SHM: req/rep<br/>sim/clock/**| TA
        Z <-->|TCP/SHM: pub/sub<br/>sim/coord/**| DC
        Z <-->|TCP: pub/sub (FlatBuffers)<br/>sim/physics/**| PG
        
        PG <-->|SHM + Futex<br/>/dev/shm/virtmcu_physics_0| PE
        
        Z -->|TCP: Telemetry Stream| OBS
        OBS -->|HTTP/WS| UI
        Q0 -.->|TCP: QMP (JSON)| UI
    end
```

**Key Characteristics (Zenoh):**
*   **Routing:** Asynchronous Publisher/Subscriber model. The broker (Zenoh Router) handles all message routing based on String topics (e.g., `firmware/control/0`).
*   **Serialization:** High overhead. Payloads are wrapped in `ZenohFrameHeader` and FlatBuffers, then serialized for network transport even when processes share the same host.
*   **Physics Loop:** The Time Authority uses Zenoh pub/sub to send `PhysicsTrigger` (FlatBuffer) to the Gateway, adding broker latency to the critical path.
*   **Race Conditions:** Because delivery is asynchronous, complex software barriers (like `ensure_session_routing()`) must be implemented to prevent packets from dropping before subscribers are ready.

### Option B: Single-Host Native IPC (New / Hybrid)

```mermaid
graph TD
    subgraph "Single Workstation"
        Q0["QEMU 0 (VirtMCU)"]
        Q1["QEMU 1 (VirtMCU)"]
        TA["Time Authority"]
        DC["Deterministic Coordinator"]
        PG["Physics Gateway"]
        PE["MuJoCo (Physics Engine)"]
        UI["NextJS UI"]
        OBS["Observability / OTel"]

        TA <-->|UDS: ClockAdvanceReq| Q0
        TA <-->|UDS: ClockAdvanceReq| Q1
        
        Q0 <-->|UDS: CoordDoneReq / Net Frames| DC
        Q1 <-->|UDS: CoordDoneReq / Net Frames| DC

        TA <-->|UDS: PhysicsTrigger (FlatBuffers)| PG
        
        PG <-->|SHM + Futex<br/>/dev/shm/virtmcu_physics_0| PE
        
        Q0 -->|UDS: Telemetry (FlatBuffers)| OBS
        PG -->|UDS: Telemetry (FlatBuffers)| OBS
        OBS -->|HTTP/WS| UI
        
        Q0 -.->|UDS: QMP (JSON)| UI
    end
```

**Key Characteristics (Native IPC):**
*   **Routing:** Explicit Point-to-Point. No broker exists. The Time Authority and Coordinator act as centralized socket servers, natively enforcing the PDES barrier.
*   **Serialization:** Extremely low overhead. The OS kernel copies data directly from sender to receiver memory without network stack traversal (TCP/IP bypass).
*   **Physics Loop:** The Time Authority sends the `PhysicsTrigger` FlatBuffer directly to the Gateway over a UDS socket (1-3µs latency). The Gateway and MuJoCo continue to use the SHM + Futex doorbell for 0% CPU lock-step execution.
*   **No Race Conditions:** UDS provides guaranteed, ordered delivery. The kernel manages backpressure, eliminating the need for application-level routing synchronization.

## Rationale & Rejected Alternatives

During research, we considered unifying all IPC over a single mechanism. The following approaches were analyzed and rejected:

### Rejected Alternative 1: 100% Shared Memory (SHM)
*What if we sent Ethernet, UART, and clock events over SHM just like physics data?*

**Why it fails: State vs. Events**
SHM is mathematically designed for **State** (the current value of a sensor array at a specific moment). It is not designed for **Events** (a chronological stream of network packets). To send an Ethernet stream over SHM, we would have to manually implement complex, lock-free ring buffers, manage head/tail pointers, and handle queue-full backpressure logic. 
Furthermore, process lifecycles are difficult in SHM. If a QEMU node crashes, its UDS socket safely closes (notifying the Coordinator instantly via `EOF`). In an SHM-only model, the memory block remains, potentially causing the Coordinator to deadlock waiting for a dead node. The Linux kernel's UDS implementation provides perfect queuing, ordering, and lifecycle management out of the box.

### Rejected Alternative 2: Eclipse Iceoryx
*Iceoryx is the automotive/ROS 2 industry standard for zero-copy SHM messaging. Why not replace Zenoh with Iceoryx?*

**Why it fails: PDES Barrier Compatibility & Orchestration Overhead**
While Iceoryx is exceptional for moving gigabytes of point-cloud data autonomously, it fundamentally conflicts with VirtMCU's deterministic constraints:
1. **Zero-Copy vs. Time-Sorting:** Iceoryx provides immediate, zero-copy delivery of messages. VirtMCU's Parallel Discrete Event Simulation (PDES) requires messages to be held, buffered, sorted by `vtime_ns`, and delivered synchronously at a quantum boundary. To do this with Iceoryx, the Coordinator would have to break the zero-copy promise, copy the data, sort it, and republish it—negating the library's primary benefit.
2. **Asynchronous vs. Lock-step:** Iceoryx is an asynchronous Pub/Sub broker. VirtMCU physics requires a synchronous, blocking handshake (the `futex` doorbell) to guarantee MuJoCo completes its step before the next firmware instruction executes. Iceoryx does not natively support this synchronous doorbell model.
3. **Daemon Overhead:** Iceoryx requires a central memory management daemon (`RouDi`). Running this daemon for the sake of passing 24-byte FlatBuffers adds immense configuration complexity to local Docker Compose environments compared to VirtMCU's zero-dependency UDS and `/dev/shm` implementation.

## Consequences
- **Positive:** Massive reduction in latency and jitter for single-machine simulation. Eradication of Zenoh pub/sub race conditions.
- **Positive:** Simplified process topology for developers running VirtMCU locally.
- **Negative:** The `DeterministicCoordinator` must be refactored to support a UDS server mode alongside its existing Zenoh subscription mode. QEMU network plugins (`netdev`, `chardev`) require an update to support UDS streaming.


