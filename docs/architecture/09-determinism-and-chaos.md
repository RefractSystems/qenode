# Chapter 9: Determinism & Chaos Engineering

## The Determinism Invariant

In VirtMCU, determinism is not a feature; it is a fundamental invariant. 

**The Invariant**: Two simulation runs with identical world configuration, firmware ELFs, and stochastic seeds produce bit-identical results — including identical UART output, identical network logs, and identical actuator command sequences — regardless of the host machine's performance or load.

---

## 1. The Global Determinism Oracle

VirtMCU enforces determinism through three coordinated mechanisms:

### Stochastic Seeding
Any simulation logic that requires randomness (e.g., CSMA/CA backoff, radio noise models, or BLE advertising slot selection) MUST NOT use system time or local PIDs for seeding. Instead, they must use:
`VirtMCU_api::seed_for_quantum(global_seed, node_id, quantum_number)`
This ensures that the "random" behavior is perfectly reproducible for every node and every quantum across multiple runs.

### Canonical Message Ordering
Inter-node messages are not delivered in arrival order. The `DeterministicCoordinator` buffers all messages within a quantum and sorts them by a global total order:
`(delivery_vtime_ns, source_node_id, sequence_number)`
This ensures that even if Node A's messages arrive at the host CPU before Node B's in one run, and vice versa in the next, the simulator always delivers them in the same deterministic sequence.

---

## 2. The ARCH-8 Barrier Protocol

To maintain causal integrity, VirtMCU uses the **ARCH-8 barrier protocol** (first introduced in Chapter 2) that prevents "clock drift" between the physics engine and the emulated nodes.

1.  **TA Advance**: TimeAuthority (TA) requests a clock advance.
2.  **Execution**: Nodes run their firmware for the requested quantum $Q$.
3.  **Completion**: Nodes send outbound messages and a `done` signal to the coordinator.
4.  **Barrier Wait**: The coordinator waits for all nodes to finish quantum $Q$.
5.  **Delivery**: The coordinator sorts and delivers all buffered messages.
6.  **Release**: The coordinator signals `start` for the next quantum $Q+1$.
7.  **Clock Ack**: The node's clock device releases the reply back to the TA.
8.  **Progression**: The TA advances the simulation to $Q+1$.

This barrier ensures that firmware in quantum $Q+1$ **always** sees all messages sent by its peers in quantum $Q$.

---

## 3. Chaos Engineering (DET-3)

While determinism provides stability, real-world networks are chaotic. VirtMCU supports **Chaos Engineering** through transport-layer jitter profiles.

Through the `DataTransport` abstraction, VirtMCU can inject:
- **Packet Loss**: Deterministically drop frames based on a reproducible seed.
- **Asymmetric Latency**: Introduce virtual-time delays to simulate long-distance links.
- **Bus Degradation**: Simulate physical bus collisions or RF interference.

Because these faults are injected based on the simulation's deterministic seeds, they are perfectly reproducible. This allows engineers to "record" a specific network failure and replay it exactly to debug firmware resilience.
