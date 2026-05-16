# Appendix C: Request for Comments (RFCs)

This index tracks the fundamental design decisions and proposals that shape the VirtMCU architecture. Each record explains the context of a decision, the alternatives considered, and the final rationale.

As VirtMCU approaches its 1.0 release, we have transitioned from an internal "Architectural Decision Record" (ADR) model to an open RFC model. This change reflects our commitment to community-driven design. We welcome feedback, alternative proposals, and rigorous debate on these foundational concepts to help drive the framework forward.

### Numbering Strategy
VirtMCU uses non-contiguous RFC numbers based on the **GitHub Pull Request number** where the RFC is proposed. This guarantees unique IDs and ensures that the RFC number acts as a permanent pointer to the original technical debate. 

| ID | Title | Status |
|---|---|---|
| [RFC-0001](0001-project-vision-and-core-constraints.md) | Project Vision, Target Audience, and Core Constraints | **Accepted** |
| [RFC-0002](0002-ai-collaboration-and-agent-mandates.md) | AI Collaboration and Agent Mandates | **Accepted** |
| [RFC-0003](0003-rust-idioms-and-community-alignment.md) | Rust Idioms and Community Alignment | **Proposed** |
| [RFC-0006](0006-binary-fidelity.md) | Binary Fidelity: The Non-Negotiable Constraint | **Accepted** |
| [RFC-0010](0010-platform-description-format.md) | Platform Description Format (YAML vs. REPL) | **Accepted** |
| [RFC-0011](0011-zenoh-federation-bus.md) | Zenoh as the Simulation Federation Bus | **Accepted** |
| [RFC-0012](0012-data-serialization.md) | Data Serialization (FlatBuffers vs. Raw Structs) | **Accepted** |
| [RFC-0013](0013-language-selection-and-native-migration.md) | Rust as the Primary Language for Core Infrastructure | **Accepted** |
| [RFC-0015](0015-logging.md) | Deterministic Logging and Observability | **Accepted** |
| [RFC-0016](0016-logical-domain-model.md) | Logical Domain Model for World Specification | **Accepted** |
| [RFC-0017](0017-sensor-data-replay.md) | Enterprise Sensor Data Replay & Trace Formats | **Accepted** |
| [RFC-0018](0018-safe-peripheral-bql-yielding.md) | Safe Peripheral BQL Yielding | **Accepted** |
| [RFC-0019](0019-single-host-native-ipc.md) | Native IPC Hybrid Architecture for Single-Host Co-Simulation | **Accepted** |
| [RFC-0020](0020-deterministic-test-orchestration-seeding.md) | Deterministic Test Orchestration Seeding | **Accepted** |
| [RFC-0021](0021-peripheral-design-and-synchronization.md) | Unified Peripheral Design and Deterministic Synchronization | **Proposed** |
| [RFC-0022](0022-fail-loudly-and-panic-linting.md) | Fail Loudly vs Linting Policy | **Accepted** |
| [RFC-0030](0030-qemu-patch-strategy.md) | QEMU Patch Strategy and "No Fork" Policy | **Accepted** |
| [RFC-0031](0031-di-and-raii-mandate.md) | Global State, Dependency Injection (DI), and RAII Mandate | **Accepted** |
| [RFC-0040](0040-testing-pyramid-and-emulation-verification.md) | The Testing Pyramid and Emulation Verification | **Accepted** |
| [RFC-0023](0023-safe-qom-macros.md) | Safe QOM Macros and Boilerplate Eradication | **Accepted** |
| [RFC-0024](0024-deterministic-routing-and-flow-control.md) | Assertion-Based Deterministic Routing and Flow Control | **Accepted** |
| [RFC-0025](0025-zero-copy-transport.md) | Zero-Copy Deterministic Transport API | **Accepted** |
| [RFC-0026](0026-zero-unsafe-qom-peripherals.md) | Zero Unsafe QOM Peripherals | **Accepted** |
| [RFC-0027](0027-cosim-bridge-raii-framework.md) | CoSimBridge RAII IoC Framework | **Accepted** |
| [RFC-0028](0028-mmio-socket-bridge-protocol.md) | MMIO Socket Bridge Architecture | **Accepted** |
| [RFC-0029](0029-remote-port-systemc-integration.md) | Remote Port (SystemC) Co-Simulation Backbone | **Accepted** |
| [RFC-0032](0032-unified-rust-build-system.md) | Unified Rust Build System (xtask) | **Accepted** |
| [RFC-0033](0033-uds-coordinator-wire-protocol.md) | UDS Coordinator Wire Protocol | **Accepted** |

> Note: RFC-0011 (Zenoh as Federation Bus) is **Accepted but partially superseded** by RFC-0019 for single-host deployments. Zenoh remains canonical only for cross-host federation; UDS is the default single-host transport.
