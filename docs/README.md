# The VirtMCU Bible: Engineering Deterministic Worlds

> "If it is not deterministic, it is not a simulation. It is a roll of the dice."

If you are reading this, you have likely spent days chasing a Heisenbug on a physical testbench. You know the pain of modern hardware development: physical silicon is slow to iterate on, expensive to deploy, and unforgiving of mistakes. 

Traditional emulators force an unacceptable compromise:
1. **Speed without flexibility:** Fast C-based emulators that require core source modifications for every new sensor.
2. **Flexibility without scale:** Interpreted simulators that cannot sustain complex, multi-node networks in real-time.

**VirtMCU** is the solution. It is the definitive, high-performance, deterministic multi-node firmware simulation framework built on QEMU. 

With VirtMCU, you can boot a thousand ARM and RISC-V microcontrollers, wire them together over virtual CAN buses and RF networks, and attach their virtual PWM pins to a 3D physics engine. Crucially, this entire distributed simulation is **globally deterministic**. Every network packet, every CPU cycle, and every physics frame happens in perfect, lockstep synchronization. Run it today, run it next year, and you will get the exact same bit-for-bit result.

---

## The Core Mandates

This book is governed by two immutable laws that define every architectural decision in VirtMCU:

### I. Binary Fidelity
**The same firmware ELF that runs on a real MCU must run unmodified in VirtMCU.** No virtmcu-specific startup code, no special linker sections, and no guest-visible MMIO for the clock. If you have to change your firmware to run it here, it is a VirtMCU bug.

### II. Global Simulation Determinism
**Same topology + same firmware + same `global_seed` = bit-identical output.** Every time. This isn't just about the CPU; it's about the network, the physics, and the stochastic noise. We eliminate the host OS from the timing equation entirely.

---

## Who This Book Is For

This is the definitive engineering manual for systems software engineers, firmware developers, and infrastructure architects. 

We assume proficiency in C, working knowledge of Rust, and a solid grasp of operating system internals. This is not a "getting started" guide for hobbyists—it is a technical specification for building enterprise-grade cyber-physical validation systems.

---

## Navigating the Matrix

*   **[Part I: Silicon Foundations](fundamentals/volume-i-intro.md)** - From SoC anatomy to Device Trees. The physical blueprint of the virtual world.
*   **[Part II: The Mechanics of Virtual Time](architecture/volume-ii-intro.md)** - Solving the causality problem. The temporal core, PDES barriers, and the deterministic transport layer.
*   **[Part III: Core Architecture](architecture/00-introduction.md)** - The internals of QEMU, the QOM object model, and the peripheral subsystem.
*   **[Part IV: Applied Simulation](tutorials/volume-iv-intro.md)** - Hands-on tutorials for building native Rust peripherals and automating multi-node labs.
*   **[Part V: Distributed & Cyber-Physical Systems](architecture/volume-v-intro.md)** - Bridging software and physics. SAL/AAL abstractions and AI-augmented debugging.
*   **[Part VI: Production Readiness](guide/volume-vi-intro.md)** - The engineering mandates. CI/CD, testing strategies, and the debugging playbook.
