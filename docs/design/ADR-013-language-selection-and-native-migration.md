# ADR-013: Language Selection Policy and Native Zenoh Migration

**Status:** Implemented
**Date:** 2026-04-16

## Context

The `virtmcu` project utilizes a mix of C, C++, Rust, and Python. As the project matures and integrates more deeply with high-performance simulation requirements (FirmwareStudio), the overhead of inter-process communication and the fragility of FFI bindings have become critical bottlenecks. Specifically, the previous C implementation of Zenoh plugins (`hw/zenoh/`) relied on `zenoh-c` bindings, which complicated the build process and introduced potential safety risks in multi-threaded contexts.

## Decision

We established a strict **Language Selection Policy** and initiated a **Native Rust Migration** for core Zenoh components.

### 1. Language Selection Policy

| Component Type | Permitted Language(s) | Rationale |
| :--- | :--- | :--- |
| **Hot-Path Simulation Loop** (Clock, MMIO, Netdev) | **Rust** (Preferred) or **C** | Must be native QOM plugins. Zero process-boundary overhead. |
| **Hardware Modeling (SystemC)** | **C++** | Native integration with SystemC TLM-2.0 framework. |
| **Offline Tooling** (Parsing, Generation) | **Python** | Rapid development, rich library ecosystem (YAML, regex). |
| **Test Orchestration** | **Python** | Integration with Pytest, Robot Framework, and QMP. |
| **Telemetry & Observability** | **Rust** | Performance, safe concurrency, and schema-driven serialization. |

**Strict Bans:**
*   **NO Python** in the simulation loop (managing MMIO, virtual time, or packet delivery).
*   **NO heavy FFI** for components that are natively available in another project-supported language (e.g., use Rust for Zenoh).

### 2. Phase 18 & 19: Native Rust Migration (The "Oxidization")

We migrated the core `hw/` subsystem and infrastructure to native Rust.

#### Tasks (Completed):
*   **18.1: Rust-QEMU Plugin Infrastructure**
    *   Stabilized the `hw/rust-dummy/` pattern into a reusable crate for QOM device registration in Rust.
    *   Enabled `meson` to compile and link Rust plugins into the QEMU binary as `.so` modules.
*   **18.2: Native Zenoh-Clock in Rust**
    *   Rewrote `zenoh-clock.c` in Rust using the `zenoh` crate.
    *   Implemented BQL (`Big QEMU Lock`) management using Rust's safety patterns.
*   **18.3: Native Zenoh-Netdev in Rust**
    *   Migrated `zenoh-netdev.c` to Rust.
    *   Replaced the C priority queue with a native Rust `BinaryHeap` for virtual-time delivery.
*   **18.4: Native Zenoh-Chardev in Rust**
    *   Migrated `zenoh-chardev.c` to Rust.
*   **18.5: Native Zenoh-Telemetry in Rust**
    *   Migrated `zenoh-telemetry.c` to Rust and integrated directly with the FlatBuffers schema.

## Status Update (2026-04-24)
Phase 18 and 19 have been successfully completed. All core plugins are now native Rust QOM modules, leveraging the `virtmcu-qom` abstraction layer.

## Consequences

*   **Build Simplification:** Removes the need to build `zenoh-c` from source. `cargo` manages Zenoh dependencies.
*   **Safety:** Drastically reduces the risk of deadlocks and memory corruption in the multi-threaded Zenoh/QEMU boundary.
*   **Performance:** Eliminates the C FFI layer for Zenoh operations.
*   **Deployment:** Eliminates the complex `LD_LIBRARY_PATH` requirement for `libzenohc.so`.

## Implementation Guidance

*   All new hardware models that require external networking MUST be written in **Rust**.
*   Existing C models should only be modified for bug fixes; new features should trigger a migration to Rust.
*   Python scripts must remain strictly "out-of-band" (pre-boot or post-mortem).
