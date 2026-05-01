# Chapter 1: The Build System

## Build Architecture Overview

VirtMCU is a bifurcated system combining a C-based emulator (QEMU) with modular, dynamic peripherals (QOM plugins) written primarily in Rust. To maintain rapid development cycles while ensuring memory safety, the build system employs a sophisticated strategy of symlinking, dynamic library loading (`dlopen`), and environment-isolated build artifacts.

---

## 1. Build Domains

The codebase is divided into three primary build domains, each with its own toolchain and lifecycle.

### A. Dependencies (C/C++)
*   **Components**: `zenoh-c` (for native plugin communication) and `flatcc` (for telemetry serialization).
*   **Lifecycle**: Built once during `make setup-initial` or pre-compiled into the project's Docker base image.
*   **Location**: Land in `third_party/zenoh-c` and `third_party/flatcc`.

### B. QEMU Core & Rust Plugins
*   **Components**: The patched `qemu-system-arm/riscv` binaries and the dynamic peripheral models (`hw-virtmcu-*.so`).
*   **Lifecycle**: 
    *   The **Core** is built initially via `make setup-initial` and rarely changes.
    *   **Plugins** are rebuilt frequently via `make build` whenever `hw/rust/` code is modified.
*   **Mechanism**: The project's `hw/` directory is **symlinked** into QEMU's source tree (`third_party/qemu/hw/virtmcu`). Incremental builds delegate to QEMU's Meson system, which in turn invokes `cargo` to compile the Rust components.

### C. Host Orchestration Tools (Rust/Python)
*   **Components**: Python test suites (`pytest`), Rust coordinators (`deterministic_coordinator`), and bridges.
*   **Lifecycle**: Built on-demand when running integration tests or via standard `cargo build` in the workspace root.
*   **Location**: Artifacts land in the standard workspace `target/` directory.

---

## 2. The Dual-Output Strategy (Standard vs. ASan)

To prevent cache thrashing when switching between standard development and AddressSanitizer (ASan) debugging, VirtMCU isolates build outputs based on the `VIRTMCU_USE_ASAN` environment variable.

| Mode | Environment | Output Directory |
|---|---|---|
| **Standard** | `VIRTMCU_USE_ASAN=0` | `third_party/qemu/build-virtmcu` |
| **Sanitized** | `VIRTMCU_USE_ASAN=1` | `third_party/qemu/build-virtmcu-asan` |

*   **Standard Build**: Optimized for developer iteration speed.
*   **ASan Build**: Compiles QEMU with `--enable-asan --enable-ubsan` and Rust plugins with `-fsanitize=address`.

---

## 3. Runtime Linking: The Plugin Lifecycle

Because VirtMCU relies on QEMU's dynamic module system, **Rust plugins do not link against QEMU at compile time; QEMU links against them at runtime.**

1.  **Discovery**: The `scripts/run.sh` script searches for the most recently modified `.so` plugins in the active build directory.
2.  **Environment Injection**: The script sets `QEMU_MODULE_DIR` to the directory containing the plugins and configures `LD_LIBRARY_PATH` for dependency resolution.
3.  **Loading (`dlopen`)**: When the guest Device Tree (DTB) requests a VirtMCU device, QEMU uses `dlopen()` to load the corresponding `.so` plugin, registers the QOM types, and instantiates the peripheral.

---

## 4. Build System Invariants

1.  **Strict Endianness**: All cross-language serialization must use explicit Little-Endian (`.to_le_bytes()`) to ensure the build is portable across host architectures.
2.  **Schema-First**: Any change to simulation wire protocols MUST start with a modification to `core.fbs`. The build system enforces that all components are compiled against the same FlatBuffers schema.
3.  **No Unannotated Sleeps**: The build includes a linter that rejects any `thread::sleep` or `asyncio.sleep` not explicitly marked as a `SLEEP_EXCEPTION`, preventing non-determinism from creeping into the codebase.
