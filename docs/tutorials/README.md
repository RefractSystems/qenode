# Part III: Practical Mastery

Welcome to the **VirtMCU** educational series. This curriculum is designed to take you from a basic understanding of machine emulation to mastering complex, multi-node deterministic simulations.

## Curriculum Overview

### Core: The Basics (The Single Node)
In this section, we learn how to run a single machine and understand the internals of how firmware interacts with virtual hardware.

*   **[Lesson 1: Dynamic Machines](./lesson1-dynamic-machines/README.md)**: Construct a virtual ARM machine from a text file and use GDB to inspect it.
*   **[Lesson 18: The MMIO Lifecycle](./lesson18-mmio-lifecycle/README.md)**: Follow the "story of a byte" from firmware instruction to QEMU trap.
*   **[Lesson 19: Native Rust Migration & Safety](./lesson19-native-rust-migration/README.md)**: Learn how we cross the FFI boundary and manage the BQL.
*   **[Lesson 2: Dynamic QOM Plugins](./lesson2-dynamic-plugins/README.md)**: Build your first peripheral in Rust without touching QEMU source.
*   **[Lesson 3: Parsing Platform Descriptions (.repl)](./lesson3-repl2qemu/README.md)**: Translate Renode `.repl` files into QEMU Device Trees.
*   **[Lesson 4: Emulation Test Automation](./lesson4-emulation-automation/README.md)**: Automate your verification with QMP and Pytest.

### System: Distributed Systems (The Multi-Node World)
We expand beyond a single CPU to a synchronized network of nodes.

*   **[Lesson 6: Multi-Node Networking](./lesson6-multi-node/README.md)**: The foundations of deterministic communication between nodes.
*   **[Lesson 7: Zenoh Clock](./lesson7-zenoh-clock/README.md)**: Slaving QEMU to an external master clock for perfect synchronization.
*   **[Lesson 8: Interactive UART](./lesson8-interactive-uart/README.md)**: Deterministic serial communication and human-in-the-loop debugging.
*   **[Lesson 5: Co-simulation](./lesson5-cosimulation/README.md)**: Connecting QEMU to external hardware models over Unix sockets.
*   **[Lesson 9: SystemC CAN](./lesson9-systemc-can/README.md)**: Building complex SystemC adapters for shared-media protocols.
*   **[Lesson 11.3: Remote Port Co-Simulation](./lesson11.3-remote-port/README.md)**: Using industry-standard protocols for Verilator and FPGA fabrics.

### Advanced: Cyber-Physical Systems
Integrating the cyber world of firmware with the physical world of sensors and actuators.

*   **[Lesson 10: The Cyber-Physical Bridge (SAL/AAL)](./lesson10-sal-aal/README.md)**: Translating between binary registers and continuous physics.
*   **[Lesson 13: AI-Augmented Debugging](./lesson13-ai-debugging/README.md)**: Using AI agents to analyze traces and generate tests.
*   **[Lesson 11: RISC-V Expansion](./lesson11-riscv-expansion/README.md)**: Heterogeneous simulation across different CPU architectures.
*   **[Lesson 11.2: Virtual-Time Timeouts](./lesson11.2-virtual-time-timeouts/README.md)**: Advanced synchronization techniques for high-load environments.

### Validation: Production Readiness
Packaging, performance, and security for enterprise-grade digital twins.

*   **[Lesson 15: Distribution & Packaging](./lesson15-distribution/README.md)**: Creating portable Docker images and binary releases.
*   **[Lesson 16: Performance & Benchmarking](./lesson16-performance/README.md)**: Measuring IPS and ensuring timing determinism.
*   **[Lesson 17: Security Boundaries](./lesson17-security-boundary/README.md)**: Fuzzing the network and protecting the simulation fabric.
