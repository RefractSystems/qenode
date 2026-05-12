# VirtMCU Tools

This directory contains a suite of utilities for hardware description, protocol handling, debugging, and co-simulation within the VirtMCU ecosystem.

## Core Utilities

### Hardware Description
*   **`virtmcu-cli platform generate`**: The primary tool for translating the modern YAML hardware description into a QEMU Device Tree (.dtb) and CLI arguments. (Formerly `yaml2qemu.py`).
*   **`repl2yaml.py`**: A migration utility used to convert legacy Renode `.repl` files into the modern YAML schema.
*   **`virtmcu-cli platform generate-header`**: Generates C++ address map headers (`.hpp`) from YAML board descriptions, ensuring C++ consumers (like SystemC adapters) stay in sync with the hardware model. (Formerly `usd_to_virtmcu.py`).
*   **`repl2qemu/`**: A Python package providing the parser and FDT (Flattened Device Tree) emitter used by `yaml2qemu.py`.

### Protocol & Bindings
*   **`vproto.py`**: Provides Pythonic, high-level wrappers around the core FlatBuffers-generated protocols. **Note: Manual use of `struct pack/unpack` is discouraged in favor of this utility.**
*   **`virtmcu/core/`**: Auto-generated Python bindings for the core VirtMCU FlatBuffers schemas.
*   **`telemetry_fbs/`, `flexray_fbs/`, `lin_fbs/`**: Auto-generated Python bindings for domain-specific FlatBuffers protocols (Telemetry, FlexRay, LIN).

## Simulation & Co-simulation

### Bridging & Coordination
*   **`deterministic_coordinator/`**: A Rust-based multi-node coordinator that uses Zenoh as the transport layer for virtual wires.
*   **`deterministic_coordinator/`**: A specialized coordinator designed for fully deterministic multi-node simulations.
*   **`cyber_bridge/`**: The core bridge implementation for connecting virtual peripherals to physical or external simulators.
*   **`systemc_adapter/`**: A C++ adapter allowing SystemC modules to participate in VirtMCU simulations via the `mmio-socket-bridge`.
*   **`virtmcu-cli fake-adapter`**: A simple mock for testing the MMIO socket protocol. (Formerly `fake_adapter.py`).

### Inspection & Telemetry
*   **`virtmcu-cli qmp`**: An interactive CLI tool for inspecting a running QEMU instance via the QEMU Machine Protocol (QMP). Essential for verifying device trees and object hierarchies. (Formerly `qmp_probe.py`).
*   **`virtmcu-cli telemetry`**: A Zenoh-based utility that subscribes to and displays real-time telemetry trace events from a simulation. (Formerly `telemetry_listener.py`).

## Testing & Debugging

### Frameworks
*   **`virtmcu-test-runner`**: The primary Rust-based test orchestration engine. It manages the lifecycle of QEMU nodes, coordinators, and bridges for both unit and integration tests.
*   **`tests/native_integration/`**: The location for high-level integration tests written in Rust using `tokio`.

### Debugging Helpers
*   **`debug/`**: GDB Python scripts for deep inspection of QEMU internal state and QOM structures.
*   **`ffi_layout_check/`**: A utility to verify that C and Rust struct layouts match.
*   **`virtmcu-coverage`**: Analyzes guest code coverage by mapping `drcov` trace files to ELF symbols.
*   **`wireshark/`**: Plugins and dissectors for live observability of Zenoh-based simulation traffic.
