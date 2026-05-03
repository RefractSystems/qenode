# How to Use This Book

Welcome to VirtMCU. Because this project bridges the worlds of C-based emulation, Rust-based peripheral modeling, Python orchestration, and distributed systems theory, there is no single "right" way to read this book. 

Instead of reading cover-to-cover, we recommend picking a path that matches your current engineering background and immediate goals.

---

## Recommended Learning Paths

### Path 1: The Software Engineer (New to Hardware)
You understand concurrency and high-level systems, but you've never had to read a hardware datasheet or flip a specific bit in a register.
1. **Part I: Silicon Foundations**: Start with SoC anatomy, MMIO, and Device Trees to understand how hardware actually works.
2. **Tutorials (Lessons 1 & 18)**: Boot your first virtual machine and follow the lifecycle of a single MMIO byte.
3. **Part III: Core Architecture**: Learn how QEMU executes code and translates it on the fly.

### Path 2: The Embedded Firmware Engineer (New to QEMU)
You can write bare-metal C in your sleep, but the idea of writing an emulator plugin or navigating the Big QEMU Lock (BQL) is new.
1. **Part III: Core Architecture**: Focus heavily on QEMU's internals and the QEMU Object Model (QOM).
2. **Tutorials (Lessons 2 & 19)**: Skip the hardware basics and jump straight into writing your first native Rust peripheral plugin.
3. **Part VI: Production Readiness**: Read up on how to write Python-based integration tests for your firmware.

### Path 3: The Distributed Systems Architect
Your primary concern is determinism, virtual time synchronization, and orchestrating complex cyber-physical co-simulations.
1. **Part II: Mechanics of Virtual Time**: Master Parallel Discrete Event Simulation (PDES) theory and the Causality Problem.
2. **Part V: Distributed & Cyber-Physical Systems**: Dive into the Temporal Core, Zenoh transport, and SAL/AAL integration.
3. **Part VI: War Stories (Postmortems)**: Read how non-determinism and silent failures manifest in real-world distributed emulators.

---

## VirtMCU Engineering Principles

Regardless of your path, these three principles govern everything we do in VirtMCU:

### 1. Binary Fidelity
The firmware ELF that programs a physical silicon wafer must run **unmodified** in VirtMCU. We do not tolerate "simulation-only" `#ifdef` macros in firmware. If the firmware has to know it is in a simulation, the simulator has failed.

### 2. Global Determinism
Given the same seed and the same network topology, the output must be bit-identical. Every time. Determinism isn't an optional flag; it's the fundamental invariant of the VirtMCU universe.

### 3. Fail Loudly
If a check can be automated (linting, FFI export verification, address alignment), it must be enforced in CI. We don't rely on developer memory. A silent failure in a simulator is worse than a crash, because it teaches the user to trust a lie.
