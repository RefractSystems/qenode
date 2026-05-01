# How to Read This Book

Welcome to the **VirtMCU** curriculum. This textbook is designed to take you from a systems software engineer to a master of deterministic firmware simulation. 

VirtMCU is a complex, multi-language project that bridges the gap between C (QEMU), Rust (Peripherals), Python (Orchestration), and Distributed Systems theory. To master this system, you must follow the path that best suits your background.

---

## Tailored Learning Paths

### If you are a Software Engineer (New to MCUs)
You likely understand concurrency and high-level languages but have never touched a register.
1.  **Fundamentals 1–4**: Learn about SoC anatomy, MMIO, and Device Trees.
2.  **Tutorials Lesson 1 & 18**: Get your hands dirty with basic simulation and the MMIO lifecycle.
3.  **Fundamentals 5–6**: Understand how QEMU actually executes your code.

### If you are an Embedded Specialist (New to QEMU)
You know your way around an STM32 but have never written an emulator plugin.
1.  **Fundamentals 5–6**: Focus heavily on QEMU's architecture and the Object Model (QOM).
2.  **Tutorials Lesson 2 & 19**: Learn how to write peripheral plugins in Rust.
3.  **Architecture 6**: Deep dive into the peripheral subsystem's concurrency model.

### If you are a Distributed Systems Researcher
You care about determinism, virtual time, and PDES.
1.  **Fundamentals 8 & 10**: Master PDES theory and the Big QEMU Lock.
2.  **Architecture 2–3**: Study the Temporal Core and the Transport Layer.
3.  **War Story (Postmortem 2026-05-01)**: See how non-determinism and silent failures manifest in the real world.

---

## The Engineering Philosophy

### 1. Fail Loudly
If a check can be automated (lint, FFI export verify, address alignment), it must be in the `Makefile` and enforced in CI. We do not rely on developer memory. A silent failure is a defect.

### 2. Binary Fidelity
The firmware ELF that runs on a real MCU must run unmodified in VirtMCU. If you have to change your C code to "make it work in simulation," VirtMCU has failed.

### 3. Global Determinism
Same seed + same topology = bit-identical output. This is our "North Star."
