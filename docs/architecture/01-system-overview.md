# System Overview

## Learning Objectives
After this chapter, you can:
1. Define "Binary Fidelity" and explain its importance in firmware validation.
2. Identify the three core pillars of the VirtMCU architecture.
3. Distinguish between the Control Plane and the Data Plane in a multi-node simulation.

## 1. What VirtMCU Is: The Cyber-Physical Architecture

VirtMCU is a **deterministic multi-node firmware simulation framework**. It serves as the cyber-emulation layer of a broader digital twin platform (like FirmwareStudio). 

To understand VirtMCU, we must first establish the high-level taxonomy of a **Cyber-Physical System (CPS)** simulation. The architecture is strictly decoupled into three abstract domains:

1. **The Physical Node**: Simulates the physical world. It calculates rigid-body dynamics, joint kinematics, environmental sensor readings, and actuator physics. It serves as the master clock for the system.
   * *Current Implementations*: We currently focus on **MuJoCo** (for high-speed kinematics) and **NVIDIA Omniverse** (for high-fidelity USD-based environments). Conceptually, this could be any physics solver.
2. **The Cyber Node**: Simulates the computing units. It executes the actual firmware binaries, providing accurate models of CPU cores, memory-mapped I/O (MMIO), and peripheral registers.
   * *Current Implementation*: We exclusively use **QEMU** (augmented with our VirtMCU Rust plugins) inside the cyber node, though the architecture could theoretically support other emulators (like Renode or Qiling) in the future.
3. **The Transport Layer**: The communication interconnect. It bridges the timing/telemetry data between Physical and Cyber nodes, and handles the emulated network traffic between multiple Cyber nodes.
   * *Current Implementations*: We support **Zenoh** (for distributed, high-throughput network topologies) and **Unix Sockets** (for low-latency, single-host IPC).

> **A Note on Terminology for the Rest of this Book**
> Throughout this book, we will often discuss the general theory of a CPS (referring to "Cyber nodes", "Physical nodes", and "Transport"). However, when we dive into implementation details, system architecture, or specific configurations, we will refer directly to our current stack: **QEMU**, **MuJoCo/Omniverse**, and **Zenoh**.

### The "Gold Standard": Binary Fidelity
The primary design constraint of the Cyber Node is **Binary Fidelity**: the same firmware ELF that programs a real microcontroller must run unmodified inside the simulator. 

To achieve this, VirtMCU mandates:
- **Exact Memory Mapping**: Peripherals are mapped at the **exact** base addresses specified in the silicon datasheet.
- **Register-Level Accuracy**: Register layouts, reset values, and interrupt numbers must match physical silicon bit-for-bit.
- **Invisible Infrastructure**: The `virtmcu-clock` and all co-simulation infrastructure are **invisible to the firmware**. They exist only at the QEMU/Plugin level and have no guest-visible MMIO exposure.
- **No Guest Hooks**: No virtmcu-specific startup code, linker sections, or compile-time flags are allowed in the firmware.

If a feature requires firmware modification to function, it is considered a bug in VirtMCU.

---

## 2. The Core Pillars

VirtMCU's architecture is built on three foundational guarantees:

### Pillar 1: Temporal Correctness
Every virtual MCU shares a synchronized notion of time. VirtMCU implements **Cooperative Time Slaving**, where the Cyber Node (QEMU) acts as a time slave to an external master clock (the Physical Node). It executes instructions at full speed within a "quantum" but pauses at every boundary until granted permission to proceed.

### Pillar 2: Global Simulation Determinism
Two simulation runs with identical inputs (firmware, topology, and stochastic seed) will produce bit-identical results. This is enforced through:
- **Declared Topology**: The full network graph is declared in the world YAML and loaded by the `DeterministicCoordinator` at startup. Runtime "scouting" or discovery is banned.
- **Canonical Tie-Breaking**: Messages with the same virtual timestamp are delivered in a strict, deterministic order: `(delivery_vtime_ns, source_node_id, sequence_number)`.
- **Stochastic Seeding**: All PRNGs are derived from a `global_seed` using the formula `seed_for_quantum(global_seed, node_id, quantum_number)`. Wall-clock seeding or `rand::thread_rng()` are strictly forbidden.

### Pillar 3: Causal Ordering
In a distributed simulation, messages must be delivered in the order they were sent in virtual time, regardless of when they arrive at the host CPU. VirtMCU's **Parallel Discrete Event Simulation (PDES)** barrier ensures that the `DeterministicCoordinator` withholds quantum-Q messages until **all** nodes signal that quantum Q is complete.

---

## 3. High-Level System Context

The diagram below illustrates how the abstract CPS concepts map to our concrete implementation stack.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│  The Digital Twin World                                                                │
│                                                                                        │
│  ┌──────────────────────┐       ┌─────────────────────┐  PhysicsTrigger / PhysicsDone │
│  │  Physics Engine      │ ◄─SHM─│  Physics Gateway    │◄────────────────────────────┐ │
│  │  (any implementation)│ ──SHM►│  (virtmcu-physics-  │                             │ │
│  └──────────────────────┘       │   gateway)          │                             │ │
│                                 └─────────────────────┘                             │ │
│                                                                                      │ │
│                                 ┌─────────────────────────────────┐                 │ │
│                                 │  Time Authority                  │─────────────────┘ │
│                                 │  (virtmcu-time-authority)        │                   │
│                                 │  - issues ClockAdvanceReq        │                   │
│                                 │  - collects actuators per quantum│                   │
│                                 └──────────────┬───────────────────┘                   │
│                                                │                                       │
│      Control Plane Transport [Zenoh / Unix Sockets]                                   │
│      (one channel per node — direct, low-latency clock sync)                          │
│                                                │                                       │
│           ┌────────────────────────────────────┼──────────────────────────┐            │
│           │  Cyber Node 0                      │  Cyber Node 1            │            │
│           │  [QEMU + VirtMCU Rust Plugins]     │  [QEMU + Rust Plugins]   │            │
│           └──────────────┬─────────────────────┴──────────┬───────────────┘            │
│                          │  Data Plane Transport           │                            │
│                          ▼  [Zenoh]                        ▼                            │
│               ┌────────────────────────────────────────────────┐                       │
│               │  Deterministic Coordinator                     │                       │
│               │  - quantum PDES barrier synchronization        │                       │
│               │  - canonical message sorting                   │                       │
│               │  - topology enforcement                        │                       │
│               └────────────────────────────────────────────────┘                       │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

VirtMCU utilizes three distinct communication channels:

1. **Control Plane (Clock Sync)**: A 1:1 low-latency RPC channel per QEMU node for
   `ClockAdvanceReq` / `ClockReadyResp`. Carried over Unix sockets or Zenoh.
2. **Physics Plane (Co-simulation)**: The `PhysicsTrigger` / `PhysicsDone` handshake
   between the Time Authority and the Physics Gateway, plus the shared-memory (SHM)
   channel between the gateway and the physics engine. See
   [Physics Gateway](./12-physics-gateway.md).
3. **Data Plane (Emulated Comms)**: A coordinated Zenoh bus for inter-node traffic
   (Ethernet, UART, CAN, RF) with canonical deterministic ordering.

---

## See Also
*   **[PDES and Virtual Time](../fundamentals/08-pdes-and-virtual-time.md)**: The theoretical foundation of Pillar 3.
*   **[The FlexRay Case Study](../postmortem/2026-05-01-flexray-rc-11-segfault.md)**: An example of how complex multi-node interactions can fail.
