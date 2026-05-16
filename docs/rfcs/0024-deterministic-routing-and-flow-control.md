# RFC-0024: Assertion-Based Deterministic Routing and Flow Control

## Status
Accepted (enforced by `DeterministicCoordinator` — unregistered packets panic)

> Note: §4 ("Zenoh Flow Control Mandate") is transport-specific. For the UDS
> data plane introduced by RFC-0019, kernel-level socket backpressure replaces
> the Zenoh `CongestionControl::Block` knob. The general principle — every
> `DataTransport` implementation must exert backpressure rather than drop —
> is unchanged.

## Context & Problem Statement
In VirtMCU's early architecture, communication between peripherals relied on "Hope-Based" pub/sub routing. Peripherals would declare arbitrary topics (e.g., `sim/reference-peripheral/tx`), and the `DeterministicCoordinator` used a hardcoded whitelist of wildcards to forward traffic. 

This created the "Silent Drop" problem: if a developer misconfigured a topic, or if the coordinator lacked a matching wildcard, the network packet disappeared into the void without throwing an error. The guest firmware would then spin infinitely waiting for data, causing integration tests to timeout with no diagnostic information.

In a Parallel Discrete Event Simulation (PDES), communication must be explicitly causal and perfectly reliable. "Silent drops" are incompatible with the "Fail Loudly" mandate (RFC-0022).

## Research & State of the Art (SOTA)
To solve this, we analyzed how SOTA deterministic simulators handle cross-node communication:

| Framework | Routing Model | Flow Control | Error Handling for Unroutable |
| :--- | :--- | :--- | :--- |
| **SystemC (TLM)** | Direct Memory Pointers (DMI) | Quantum-based temporal decoupling. | Compilation error (sockets must be explicitly bound). |
| **FireSim** | FPGA PHASED Bridges | Token-based / Cycle-accurate buffering. | Hard crash if bridge FIFOs drop data. |
| **gem5** | Global Event Queue | Strictly sized Port queues with backpressure. | Simulation abort if an unlinked port is accessed. |
| **VirtMCU (Legacy)**| Wildcard Pub/Sub (Zenoh) | Best-effort with infinite crossbeam queues. | **Silent Drop (The Problem)** |

## Decision: Assertion-Based Routing
VirtMCU will transition to an **Assertion-Based, Point-to-Point Routing** model. We explicitly reject the flexibility of arbitrary Pub/Sub in favor of strict, validated topology enforcement.

### 1. Topology as the Absolute Source of Truth
Peripherals will no longer define their own "topics" via properties. Instead, the `DeterministicCoordinator` parses the `topology.yml` and injects exact point-to-point links into the `DataTransport`.
*   **Format:** `sim/link/<source_node>:<peripheral>/to/<target_node>:<peripheral>`

### 2. The "Fail Loudly" Routing Map
The Coordinator will abandon wildcard whitelisting. At startup, it will generate an exact **Routing Map** of all valid links. If a packet arrives at the Coordinator with a topic not in the Routing Map, the Coordinator MUST immediately `panic!("Unroutable packet detected. Topology mismatch.")`.

### 3. Pre-Flight Liveliness Barrier
Before issuing the QMP `cont` signal to start the virtual CPUs, the Coordinator must verify that every link defined in the `topology.yml` has exactly one active publisher and one active subscriber connected to the Zenoh/Unix bus. If a node fails to subscribe, the simulation refuses to start.

### 4. Zenoh Flow Control Mandate
To prevent silent drops due to wall-clock network congestion:
*   All `DataTransport` implementations MUST configure underlying sockets for `Reliability::Reliable`.
*   If using Zenoh, `CongestionControl::Block` MUST be used. The vCPU thread is already protected by the "Yield-on-Read / Async-on-Write" pattern (RFC-0021), so blocking the background publisher thread safely exerts backpressure on the simulation without deadlocking QEMU.

## Alternatives Considered & Rejected

*   **Credit-Based Flow Control (CXL-style FLITs):** We considered implementing strict Credit-Based flow control where Node A cannot send a packet unless Node B grants a credit. 
    *   *Why we rejected it:* It introduces massive overhead. Because VirtMCU runs full OS/firmware stacks that can generate thousands of interrupts per virtual millisecond, the overhead of exchanging credit tokens across a distributed Zenoh network would cripple simulation throughput. We rely on Zenoh's underlying TCP backpressure instead.
*   **Zenoh `MatchingListener`:** Zenoh provides a feature to detect if a publisher has no matching subscribers.
    *   *Why we rejected it:* It is an asynchronous notification. By the time the publisher is notified that the subscriber is missing, the packet has already been dropped, violating determinism. The Pre-Flight Liveliness Barrier guarantees safety *before* packets are sent.

## Risks and What Can Go Wrong
1.  **PDES Circular Deadlocks:** By switching to `CongestionControl::Block`, if Node A's queue is full because it is waiting for Virtual Time to advance, and Node B is blocked trying to send to Node A, the entire simulation will freeze. This will require robust "Null Message" (heartbeat) generation to advance Virtual Time and clear queues during idle periods.
2.  **Loss of Dynamic Topologies:** Strict point-to-point routing makes simulating mobile nodes (e.g., a swarm of drones moving in and out of radio range) much harder. *Mitigation:* Radio models (like `ieee802154`) will require a specialized "Ether/Physics Gateway" component that handles dynamic attenuation, rather than relying on Zenoh pub/sub to mimic physical connection loss.

## Consequences
*   **Positive:** The "Debugging Void" is eliminated. Topology errors result in immediate, highly descriptive crashes at startup.
*   **Positive:** "Silent drops" are structurally impossible, guaranteeing bit-identical determinism regardless of network conditions.
*   **Negative:** Simulation configuration becomes rigid. Developers must explicitly link every single UART/SPI pin in the YAML.

## Related
- RFC-0006: Binary Fidelity
- RFC-0021: Unified Peripheral Design
- RFC-0022: Fail Loudly and Panic Linting
