# VirtMCU Downstream Integration & Reuse Guide

VirtMCU is designed to be highly composable. If you are building a higher-level framework (like **Firmware Studio**) that wraps VirtMCU to orchestrate firmware alongside physical simulations (e.g., MuJoCo) or external cyber-systems, you can leverage our internal tools and scripts directly.

By importing these tools, you avoid duplicating complex logic related to QEMU synchronization, DTB generation, and Zenoh IPC messaging.

## 1. Simulation Orchestration & Synchronization

If your downstream project needs to keep QEMU virtual time locked to another simulation's physical time step, use the internal Python test suite modules. They implement the strict **liveliness barriers**, **PDES deterministic sync**, and **QMP interactions** required to safely drive QEMU.

*   `tools/testing/virtmcu_test_suite/simulation.py` (`Simulation` class): 
    The single-entry-point simulation harness. It handles the strict lifecycle: spawning QEMU frozen, establishing Zenoh coordination, and executing safe teardowns. Downstream orchestrators should wrap this class.
*   `tools/testing/virtmcu_test_suite/conftest_core.py` (`VirtualTimeAuthority` class): 
    Use the `VTA` to explicitly step QEMU's clock forward (e.g., `await vta.step(ns)`). If MuJoCo dictates physics steps, call this to keep QEMU perfectly synchronized.
*   `tools/testing/virtmcu_test_suite/qmp_bridge.py`: 
    Provides an asynchronous wrapper around QEMU Machine Protocol (QMP). Use it to query CPU state, registers, or inject faults programmatically.
*   `tools/testing/virtmcu_test_suite/transport.py`: 
    The `SimulationTransport` interface. Allows your physical simulation to inject sensor data (via Zenoh) directly into the simulated MCU's memory space via our plugins.

## 2. World Topology & Configuration

VirtMCU uses a strict YAML topology format. You do not need to parse this manually or figure out how to translate it to QEMU arguments.

*   `tools/yaml2qemu.py`: 
    **Core Engine**. Takes a `.yaml` or `.yml` world definition, validates it against our schema, generates the Device Tree Blob (DTB), and constructs the exact `qemu-system-arm` command-line arguments.
*   `tools/repl2yaml.py`: 
    A utility to automatically migrate legacy Renode `.repl` board configurations into VirtMCU's deterministic YAML format.
*   `scripts/check_schemas.sh` & `scripts/generate_schemas.sh`: 
    If your downstream project adds new fields to the `.tsp` schemas, use these scripts to compile TypeSpec into JSON Schemas and FlatBuffer bindings.

## 3. Protocol Serialization & IPC

*   `tools/vproto.py`: 
    Provides Pythonic `@dataclass` wrappers around our generated FlatBuffers. Use this to construct or parse the binary messages flowing between QEMU nodes or between your orchestrator and VirtMCU.
*   `scripts/probe-qemu.py`: 
    A utility leveraging `pahole` to extract ground-truth C struct layouts directly from the compiled `qemu-system-arm` binary. Vital if you are building custom FFI bridges and need to verify alignment.

## 4. Workspace & Lifecycle Management

These scripts are written to be **multi-agent safe** and **location-agnostic**. They can be run safely from a parent project to manage a VirtMCU submodule.

*   `scripts/cleanup-sim.sh`: 
    Safely kills orphaned QEMU and Zenoh processes. It checks `/proc/<pid>/cwd` to ensure it **only** touches simulations originating from the current workspace, preventing interference with parallel simulations (e.g., in MuJoCo).
*   `scripts/install-third-party.sh`: 
    The unified setup script. It clones, patches, configures, and builds our custom deterministic QEMU fork.
*   `scripts/check-versions.py` & `scripts/sync-versions.py`: 
    Use the `--root` flag to ensure your parent repository's Python (`pyproject.toml`) and Rust (`Cargo.toml`) dependencies perfectly match VirtMCU's `BUILD_DEPS` pinning.

---
*For details on utilizing our extensive linting and testing infrastructure downstream, see `scripts/testing/README.md` and `scripts/lints/README.md`.*
