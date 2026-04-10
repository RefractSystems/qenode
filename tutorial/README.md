# virtmcu Tutorials

Welcome to the **virtmcu** educational series. This folder contains hands-on tutorials designed for computer science graduate students, researchers, and engineers who want to understand the internals of machine emulation, dynamic hardware construction, and bare-metal firmware execution.

## Motivation

Standard hardware emulators like QEMU are incredibly fast but relatively rigid: modifying a simulated motherboard to add a new sensor usually requires writing C code and recompiling the emulator. Frameworks like Renode are highly flexible (using text-based `.repl` files to wire up hardware dynamically) but sacrifice performance due to cross-language (C to C#) boundaries.

**virtmcu** bridges this gap. We are modifying QEMU to be completely dynamic while retaining its native C/TCG execution speed. 

## Curriculum

*   **[Lesson 1: Dynamic Machines, Device Trees, and Bare-Metal Debugging](./lesson1-dynamic-machines/README.md)**
    Learn how to construct a virtual ARM machine from a text file, write bare-metal assembly to interact with Memory-Mapped I/O (MMIO), and use GDB to inspect the CPU state at the instruction level.

*(More lessons will be added as we continue to build out the framework!)*
