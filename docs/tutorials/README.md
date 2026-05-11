# The VirtMCU Laboratory: A Practical Curriculum

**Welcome to the VirtMCU Laboratory.**

This curriculum is designed to translate the theoretical foundations of the VirtMCU Architecture into tangible engineering mastery. As a student in this laboratory, you will progress from single-node logic to the orchestration of complex, multi-node deterministic universes.

---

## Laboratory Series I: Foundational Execution (The Cyber Node)

In this series, we focus on the **Atomic Node**. You will learn how to construct, execute, and verify a single machine instance and its peripheral models using native Rust and YAML.

*   **[Lesson 1: Dynamic Machines](./lesson01-dynamic-machines/README.md)**: Construct a virtual ARM machine from a text-based blueprint and use GDB for deep-state inspection.
*   **[Lesson 2: Dynamic QOM Plugins](./lesson02-dynamic-plugins/README.md)**: Master the creation of Dynamic Shared Objects (DSOs) for modular emulator extension.
*   **[Lesson 3: The World Specification](./lesson03-world-specification/README.md)**: Bridging ecosystem formats by authoring VirtMCU YAML Device Trees via `yaml2qemu`.
*   **[Lesson 4: The MMIO Lifecycle](./lesson04-mmio-lifecycle/README.md)**: Forensic analysis of the "story of a byte"—following an instruction from the CPU into a custom register.
*   **[Lesson 5: Rust FFI and the BQL](./lesson05-rust-ffi-and-bql/README.md)**: The SOTA standard for memory-safe peripheral modeling and RAII-based Big QEMU Lock (BQL) management.
*   **[Lesson 6: Emulation Test Automation](./lesson06-test-automation/README.md)**: Implementing rigorous verification using QMP and the `virtmcu-test-runner` framework.

---

## Laboratory Series II: Distributed Systems & Co-Simulation

In this series, we expand our horizon to the **Distributed Universe**. You will learn to interconnect nodes and bridge the digital world with physical reality over deterministic channels.

*   **[Lesson 7: Multi-Node Networking](./lesson07-multi-node-networking/README.md)**: The foundations of deterministic, lockstep communication between heterogeneous nodes.
*   **[Lesson 8: Zenoh Clock](./lesson08-zenoh-clock/README.md)**: Slaving the emulator to a high-speed master clock for perfect synchronization across distributed clusters.
*   **[Lesson 9: Interactive UART](./lesson09-interactive-uart/README.md)**: Maintaining temporal determinism during interactive human-in-the-loop debugging.
*   **[Lesson 10: Hardware Co-simulation](./lesson10-hardware-cosimulation/README.md)**: Connecting the emulator to external hardware models via low-latency Unix sockets.
*   **[Lesson 11: SystemC CAN](./lesson11-systemc-can/README.md)**: Synthesizing complex SystemC adapters for shared-media protocol simulation.

---

## Laboratory Series III: Cyber-Physical Synthesis & Advanced Architecture

*   **[Lesson 12: The Cyber-Physical Bridge (SAL/AAL)](./lesson12-cyber-physical-bridge/README.md)**: The pattern for translating between binary registers and continuous physics.
*   **[Lesson 13: Virtual-Time Timeouts](./lesson13-virtual-time-timeouts/README.md)**: Advanced synchronization techniques for high-load, high-fidelity environments.
*   **[Lesson 14: RISC-V Expansion](./lesson14-riscv-expansion/README.md)**: Heterogeneous orchestration across disparate CPU architectures.

---

## Laboratory Series IV: Production Engineering & Validation

*   **[Lesson 15: Performance & Benchmarking](./lesson15-performance-profiling/README.md)**: Quantifying Instructions-Per-Second (IPS) and ensuring timing integrity under load.
*   **[Lesson 16: Security Boundaries](./lesson16-security-boundaries/README.md)**: Fuzzing the simulation fabric and protecting the integrity of the "Matrix."
*   **[Lesson 17: Distribution & Packaging](./lesson17-distribution/README.md)**: Architecting portable, containerized releases for enterprise-grade digital twins.
*   **[Lesson 18: AI-Augmented Debugging](./lesson18-ai-debugging/README.md)**: Utilizing the Model Context Protocol (MCP) and AI agents for forensic trace analysis.