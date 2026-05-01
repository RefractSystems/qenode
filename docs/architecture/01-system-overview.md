# Chapter 1: System Overview

## Learning Objectives
After this chapter, you can:
1. Define "Binary Fidelity" and explain its importance in firmware validation.
2. Identify the three core pillars of the VirtMCU architecture.
3. Distinguish between the Control Plane and the Data Plane in a multi-node simulation.

## 1. What VirtMCU Is

VirtMCU is a **deterministic multi-node firmware simulation framework** built on the high-performance QEMU engine. It acts as the cyber-emulation layer of **FirmwareStudio**, a digital twin platform where a physics engine (such as MuJoCo) simulates the physical world and serves as the master clock for all nodes.

### The "Gold Standard": Binary Fidelity
The primary design constraint of VirtMCU is **Binary Fidelity**: the same firmware ELF that programs a real microcontroller must run unmodified inside the simulator. This ensures that validation performed in VirtMCU is directly applicable to the physical hardware.

---

## 2. The Core Pillars

VirtMCU's architecture is built on three foundational guarantees:

### Pillar 1: Temporal Correctness
Every virtual MCU shares a synchronized notion of time. VirtMCU implements **Cooperative Time Slaving**, where QEMU acts as a time slave to an external master clock. It executes instructions at full speed within a "quantum" but pauses at every boundary until granted permission to proceed.

### Pillar 2: Global Determinism
Two simulation runs with identical inputs (firmware, topology, and stochastic seed) will produce bit-identical results. This is achieved by:
- Eliminating host-load-dependent timing.
- Enforcing canonical message ordering in the simulation bus.
- Using a centralized coordinator to synchronize node boundaries.

### Pillar 3: Causal Ordering
In a distributed simulation, messages must be delivered in the order they were sent in virtual time, regardless of when they arrive at the host CPU. VirtMCU's **Parallel Discrete Event Simulation (PDES)** barrier ensures that all nodes finish their current time quantum before any messages are delivered for the next, preserving causal integrity.

---

## 3. High-Level System Context

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  FirmwareStudio World                                                       │
│                                                                             │
│  ┌──────────────┐  mj_step()  ┌───────────────────────────────────────┐    │
│  │  MuJoCo      │ ──────────► │  TimeAuthority (Python)               │    │
│  │  (physics)   │             │  - steps all node clocks              │    │
│  │              │ ◄────────── │  - pushes topology updates            │    │
│  └──────────────┘ sensor data └───────┬───────────────────────────────┘    │
│                                       │                                     │
│               clock: ClockSyncTransport (Unix socket / Zenoh)              │
│               one channel per node — direct, low-latency                   │
│                                       │                                     │
│           ┌───────────────────────────┼──────────────────────┐             │
│           │  VirtMCU Node 0           │   VirtMCU Node 1     │             │
│           │  (QEMU + Rust Plugins)    │   (QEMU + Rust Plugins)│             │
│           └───────────┬───────────────┴───────────┬──────────┘             │
│                       │  emulated comms           │                        │
│                       ▼                           ▼                        │
│            ┌─────────────────────────────────────────┐                     │
│            │  Deterministic Coordinator              │                     │
│            │  - quantum barrier synchronization      │                     │
│            │  - canonical message sorting            │                     │
│            │  - topology enforcement                 │                     │
│            └─────────────────────────────────────────┘                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

VirtMCU utilizes two distinct communication planes:
1.  **The Control Plane (Clock Sync)**: A high-frequency, low-latency 1:1 channel for time synchronization.
2.  **The Data Plane (Emulated Comms)**: A coordinated bus for all inter-node traffic (Ethernet, UART, CAN, RF), ensuring deterministic delivery.

---

## See Also
*   **[PDES and Virtual Time](../fundamentals/08-pdes-and-virtual-time.md)**: The theoretical foundation of Pillar 3.
*   **[The FlexRay Case Study](../postmortem/2026-05-01-flexray-rc-11-segfault.md)**: An example of how complex multi-node interactions can fail.
