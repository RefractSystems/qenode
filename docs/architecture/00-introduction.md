# Part II: Architecture & Theory

## The VirtMCU Specification: A Theory of Operation

With the foundations laid and your laboratory ready, we now turn to the theoretical heart of VirtMCU. This section serves as the definitive guide to VirtMCU's design, implementation, and the physical principles that govern deterministic emulation.

VirtMCU is the world's first **deterministic multi-node firmware simulation framework** built on the high-performance QEMU engine. It achieves cycle-accurate, bit-identical reproduction of bare-metal workloads across distributed host clusters by bridging QEMU with a state-of-the-art Parallel Discrete Event Simulation (PDES) synchronization layer.

---

## Table of Contents

### 1. [System Overview](01-system-overview.md)
The high-level pillars of the architecture and the flow of data between the Control Plane (Time) and the Data Plane (Communication).

### 2. [The Temporal Core](02-temporal-core.md)
The most critical chapter. Here we discuss virtual time synchronization, the ARCH-8 barrier, and how we solve the "Causality Problem" in distributed systems.

### 3. [Transport Layer](03-transport-layer.md)
How nodes are physically connected. We explore the use of Zenoh and Unix sockets for low-latency, high-reliability message passing.

### 4. [Communication Protocols](04-communication-protocols.md)
The Data Plane: How we use FlatBuffers and schema-driven design to ensure that all nodes speak the same language, regardless of their host architecture.

### 5. [Emulator Internals](05-emulator-internals.md)
Deep dive into the engine. We explore TCG hooks, MemoryRegion routing, and how we dynamically construct a machine model from an FDT.

### 6. [Peripheral Subsystem](06-peripheral-subsystem.md)
Extending the emulator. We discuss native Rust QOM plugins, the Big QEMU Lock (BQL) safety patterns, and achieving timing fidelity in peripheral models.

---

## Guiding Design Principles

### 1. Binary Fidelity Above All
The same firmware ELF that programs a real MCU must run unmodified inside VirtMCU. If the firmware requires a special "simulation build," the simulation is incomplete.

### 2. Global Determinism
Two runs with the same world state and seed produce bit-identical results. Determinism is not an "optional feature"; it is the fundamental invariant of the system.

### 3. Zero-Latency Abstraction
Co-simulation must be fast. We avoid high-overhead IPC for hot-path MMIO, preferring native plugin execution and shared-memory where possible.
