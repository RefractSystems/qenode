# Volume III: The VirtMCU Core Architecture

## The VirtMCU Specification: A Theory of Operation

With the foundations of silicon established and the theoretical pillars of time understood, we now turn to the **VirtMCU Core Architecture**. This volume serves as the definitive specification for the system's design, implementation, and the engineering principles that enable high-performance, deterministic emulation.

VirtMCU is a unique synthesis of high-performance emulation and distributed systems theory. By leveraging the industry-standard QEMU engine and augmenting it with our synchronization temporal core, we achieve a level of fidelity previously reserved for expensive, proprietary hardware-in-the-loop (HIL) systems.

---

## Volume Contents

### 13. [System Overview: The Control and Data Planes](01-system-overview.md)
An exploration of the bifurcated architecture: the **Control Plane** (Virtual Time synchronization) and the **Data Plane** (High-speed communication).

### 15. [Emulator Internals: TCG and Memory Routing](05-emulator-internals.md)
A deep-dive into the engine's "hot loop." We explore Tiny Code Generator (TCG) hooks, MemoryRegion routing, and the dynamic construction of machines from Device Trees.

### 16. [The Peripheral Subsystem: Extending the Machine](06-peripheral-subsystem.md)
The architecture of extensibility. We discuss native Rust QOM plugins, timing fidelity in peripheral models, and our "Binary Fidelity" mandate.

### 17. [BQL and Concurrency: Safety in High-Speed Emulation](fundamentals/10-bql-and-concurrency.md)
Solving the concurrency problem. We analyze the Big QEMU Lock (BQL) safety patterns and how to prevent deadlocks in multi-threaded simulation environments.

### 18. [World Specification: Declaring the Universe](10-world-specification.md)
The topology of the Matrix. We explore how YAML is used to declare the entire simulation universe—from CPU counts to network graphs.

---

## Core Architectural Mandates

### 1. Determinism as an Invariant
In this volume, we demonstrate how determinism is baked into every layer of the architecture, from memory access to network packet delivery.

### 2. Performant Extensibility
We avoid the performance penalties of traditional IPC by executing peripheral models as native plugins within the emulator's address space.

### 3. Schema-Driven Integrity
All communication between nodes is governed by rigorous FlatBuffers schemas, ensuring that the "Neural Link" remains robust across distributed clusters.
