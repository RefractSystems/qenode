# VirtMCU Downstream Integration & Reuse Guide

VirtMCU is designed to be highly composable. If you are building a higher-level framework (like **Firmware Studio**) that wraps VirtMCU to orchestrate firmware alongside physical simulations (e.g., MuJoCo) or external cyber-systems, you can leverage our internal Rust tools directly.

By utilizing these tools, you avoid duplicating complex logic related to QEMU synchronization, DTB generation, and Zenoh IPC messaging.

## 1. Simulation Orchestration & Synchronization

If your downstream project needs to keep QEMU virtual time locked to another simulation's physical time step, use our native Rust test runner and coordinator tools. They implement the strict **liveliness barriers**, **PDES deterministic sync**, and **QMP interactions** required to safely drive QEMU.

*   `tools/virtmcu-test-runner`: 
    The single-entry-point simulation harness. It handles the strict lifecycle: spawning QEMU frozen, establishing Zenoh coordination, applying Device Trees, and executing safe teardowns. Downstream orchestrators can use its internal libraries (`virtmcu_test_runner::builder` and `virtmcu_test_runner::coordinator`) for native Rust integration.
*   `tools/deterministic_coordinator`: 
    The standalone Rust binary that enforces the per-quantum barrier synchronization. If you are orchestrating manually, launch this binary and provide it with the world topology YAML.
*   `tools/virtmcu-cli`: 
    The unified developer utility. It provides `virtmcu-cli qmp` for asynchronous QEMU Machine Protocol (QMP) interactions (querying CPU state, injecting faults) and `virtmcu-cli telemetry` for live inspection.
*   `tools/virtmcu-wire` & `tools/virtmcu-run`: 
    Native Rust APIs for interfacing with the simulation transport, allowing your physical simulation to inject sensor data (via Zenoh) directly into the simulated MCU's memory space.

## 2. World Topology & Configuration

VirtMCU uses a strict YAML topology format. You do not need to parse this manually or figure out how to translate it to QEMU arguments.

*   `tools/yaml2qemu`: 
    **Core Engine**. Takes a `.yaml` or `.yml` world definition, automatically infers addresses via SVD (System View Description), generates and validates the Device Tree Blob (DTB), and constructs the exact `qemu-system-arm` command-line arguments. It is available as a reusable Rust crate.
*   `schema/` and `scripts/generate_schemas.sh`: 
    If your downstream project adds new fields to the `.tsp` schemas, use our schema generators to compile TypeSpec into JSON Schemas and FlatBuffer bindings.

## 3. Protocol Serialization & IPC

*   `tools/vproto.py`: 
    Provides Pythonic `@dataclass` wrappers around our generated FlatBuffers for quick scripting.
*   `tools/ffi_layout_check/` & `tools/virtmcu-test-runner` (Lints): 
    We enforce exact binary alignment between Rust plugin structs and QEMU's internal C structs using native Rust lints (e.g., `QomTypeInfoLint`, `ExportLint`), eliminating the need for manual `pahole` or `gdb` probing scripts.

## 4. Workspace & Lifecycle Management

The `virtmcu-cli` tool provides a unified `setup` interface for lifecycle management. These commands are written to be **multi-agent safe** and **location-agnostic**. They can be run safely from a parent project to manage a VirtMCU submodule.

*   `cargo run -p virtmcu-cli -- setup cleanup-sim`: 
    Safely kills orphaned QEMU and Zenoh processes. It checks `/proc/<pid>/cwd` to ensure it **only** touches simulations originating from the current workspace, preventing interference with parallel simulations (e.g., in MuJoCo).
*   `cargo run -p virtmcu-cli -- setup bootstrap`: 
    The unified setup command. It clones submodules and initializes the environment.
*   `cargo run -p virtmcu-cli -- setup patch-qemu`: 
    Natively applies our deterministic QEMU patches without relying on brittle Bash or Sed scripts.
*   `cargo run -p virtmcu-cli -- setup sync-versions`: 
    Ensure your parent repository's dependencies perfectly match VirtMCU's `BUILD_DEPS` pinning.

---
*For details on utilizing our extensive linting and testing infrastructure downstream, run `cargo run -p virtmcu-test-runner -- lint --help`.*
