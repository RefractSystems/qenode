# virtmcu Tutorials

Welcome to the **virtmcu** educational series. This folder contains hands-on tutorials designed for computer science graduate students, researchers, and engineers who want to understand the internals of machine emulation, dynamic hardware construction, and bare-metal firmware execution.

## Motivation

Standard hardware emulators like QEMU are incredibly fast but relatively rigid: modifying a simulated motherboard to add a new sensor usually requires writing C code and recompiling the emulator. Frameworks like Renode are highly flexible (using text-based `.repl` files to wire up hardware dynamically) but sacrifice performance due to cross-language (C to C#) boundaries.

**virtmcu** bridges this gap. We are modifying QEMU to be completely dynamic while retaining its native C/TCG execution speed. 

## Curriculum

*   **[Lesson 1: Dynamic Machines, Device Trees, and Bare-Metal Debugging](./lesson1-dynamic-machines/README.md)**
    Learn how to construct a virtual ARM machine from a text file, write bare-metal assembly to interact with Memory-Mapped I/O (MMIO), and use GDB to inspect the CPU state at the instruction level.

*   **[Lesson 2: Dynamic QOM Plugins](./lesson2-dynamic-plugins/README.md)**
    Learn how to add entirely new peripheral devices to QEMU *without* modifying the core emulator source code by leveraging the QEMU Object Model (QOM) and dynamic shared libraries in C and Rust.

*   **[Lesson 3: Parsing Platform Descriptions (.repl) to Device Trees](./lesson3-repl2qemu/README.md)**
    Discover how to translate high-level hardware description files (like Renode's `.repl` and OpenUSD-aligned YAML) into standardized Device Tree Blobs that QEMU can boot from directly.

*   **[Lesson 4: Emulation Test Automation with QMP and Pytest](./lesson4-emulation-automation/README.md)**
    Learn how to automate the testing of your firmware and virtual hardware using the QEMU Machine Protocol (QMP), Python `asyncio`, and Robot Framework keywords.

*   **[Lesson 5: Hardware Co-Simulation — Connecting SystemC Models to QEMU](./lesson5-cosimulation/README.md)**
    Extend QEMU's MMIO subsystem to communicate with an external hardware model — specifically a SystemC TLM-2.0 register file — over a Unix domain socket.

*   **[Lesson 6: Deterministic Multi-Node Networking](./lesson6-multi-node/README.md)**
    Explore how virtmcu handles multi-node coordination with absolute determinism, replacing the traditional `WirelessMedium` typically found in Renode, and allowing multiple independent QEMU instances to communicate reliably without losing deterministic execution.

*(More lessons will be added as we continue to build out the framework!)*
