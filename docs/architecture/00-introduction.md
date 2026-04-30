# The Virtmcu Specification: A Theory of Operation for Deterministic Emulation

## Preface

VirtMCU is the world's first **deterministic multi-node firmware simulation framework** built on the high-performance QEMU engine. It provides the ergonomics of Renode with the execution speed of a JIT-accelerated emulator, bridged by a state-of-the-art Parallel Discrete Event Simulation (PDES) synchronization layer.

This specification serves as the definitive guide to VirtMCU's design, implementation, and theory of operation. It is intended for systems architects, firmware engineers, and security researchers who require a deep understanding of how VirtMCU achieves cycle-accurate, bit-identical reproduction of bare-metal workloads across distributed host clusters.

---

## Table of Contents

### Part I: The Foundation
- **[Chapter 1: System Overview](01-system-overview.md)**: Pillars of the architecture and high-level data flow.
- **[Chapter 2: The Temporal Core](02-temporal-core.md)**: Virtual time synchronization and the ARCH-8 barrier.
- **[Chapter 3: Transport Layer](03-transport-layer.md)**: Physical connectivity: Zenoh and Unix sockets.
- **[Chapter 4: Communication Protocols](04-communication-protocols.md)**: The Data Plane: FlatBuffers, Schema, and Topic Mapping.

### Part II: The Emulator
- **[Chapter 5: Emulator Internals](05-emulator-internals.md)**: TCG Hooks, MemoryRegion routing, and the FDT machine model.
- **[Chapter 6: Peripheral Subsystem](06-peripheral-subsystem.md)**: Native Rust QOM plugins, BQL safety, and Timing Fidelity.

### Part III: Cyber-Physical Integration
- **[Chapter 7: Cyber-Physical Integration](07-cyber-physical-integration.md)**: Sensor/Actuator Abstraction (SAL/AAL) and the OpenUSD Vision.
- **[Chapter 8: Observability & AI Co-pilot](08-observability-and-ai.md)**: Telemetry, MCP Server integration, and semantic debugging.
- **[Chapter 9: Determinism & Chaos Engineering](09-determinism-and-chaos.md)**: Stochastic seeding and network fault injection.

### Part IV: Reference
- **[Appendix A: ADR Index](adr/index.md)**: Historical architectural decision records.

---

## Guiding Design Principles

### 1. Binary Fidelity Above All
The same firmware ELF that programs a real MCU must run unmodified inside VirtMCU. If the firmware requires a special "simulation build," the simulation is incomplete.

### 2. Rust-First Core
All new core infrastructure and peripheral models are written in Rust. We leverage Rust's memory safety to eliminate the class of "ghost bugs" and memory corruption common in traditional C-based emulators.

### 3. Global Determinism
Two runs with the same world state and seed produce bit-identical results. Determinism is not an "optional feature"; it is the fundamental invariant of the system.

### 4. Zero-Latency Abstraction
Co-simulation must be fast. We avoid high-overhead IPC (like Python-in-the-loop) for hot-path MMIO, preferring native plugin execution and shared-memory where possible.
