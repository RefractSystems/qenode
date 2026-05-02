# VirtMCU Tools

This directory contains a suite of utilities for hardware description, protocol handling, debugging, and co-simulation within the VirtMCU ecosystem.

## Core Utilities

### Hardware Description
*   **`yaml2qemu.py`**: The primary tool for translating the modern YAML hardware description into a QEMU Device Tree (.dtb). It utilizes the `repl2qemu` package.
*   **`repl2yaml.py`**: A migration utility used to convert legacy Renode `.repl` files into the modern YAML schema.
*   **`usd_to_virtmcu.py`**: Generates C++ address map headers (`.hpp`) from YAML board descriptions, ensuring C++ consumers (like SystemC adapters) stay in sync with the hardware model.
*   **`repl2qemu/`**: A Python package providing the parser and FDT (Flattened Device Tree) emitter used by `yaml2qemu.py`.

### Protocol & Bindings
*   **`vproto.py`**: Provides Pythonic, high-level wrappers around the core FlatBuffers-generated protocols. **Note: Manual use of `struct pack/unpack` is discouraged in favor of this utility.**
*   **`virtmcu/core/`**: Auto-generated Python bindings for the core VirtMCU FlatBuffers schemas.
*   **`telemetry_fbs/`, `flexray_fbs/`, `lin_fbs/`**: Auto-generated Python bindings for domain-specific FlatBuffers protocols (Telemetry, FlexRay, LIN).
*   **`proto_gen.py`**: (*Legacy*) An older utility for generating Python bindings from C headers. Now largely superseded by FlatBuffers and `vproto.py`.

## Simulation & Co-simulation

### Bridging & Coordination
*   **`deterministic_coordinator/`**: A Rust-based multi-node coordinator that uses Zenoh as the transport layer for virtual wires.
*   **`deterministic_coordinator/`**: A specialized coordinator designed for fully deterministic multi-node simulations.
*   **`cyber_bridge/`**: The core bridge implementation for connecting virtual peripherals to physical or external simulators.
*   **`systemc_adapter/`**: A C++ adapter allowing SystemC modules to participate in VirtMCU simulations via the `mmio-socket-bridge`.
*   **`fake_adapter.py`**: A simple Python-based mock for testing the MMIO socket protocol.

### Inspection & Telemetry
*   **`qmp_probe.py`**: An interactive CLI tool for inspecting a running QEMU instance via the QEMU Machine Protocol (QMP). Essential for verifying device trees and object hierarchies.
*   **`telemetry_listener.py`**: A Zenoh-based utility that subscribes to and displays real-time telemetry trace events from a simulation.

## Testing & Debugging

### Frameworks
*   **`testing/`**: Contains Robot Framework keywords (`qemu_keywords.robot`), an asynchronous QMP bridge (`qmp_bridge.py`), and pytest fixtures for automated integration testing.
*   **`mcp_server/`**: A Model Context Protocol (MCP) server implementation, enabling AI agents to interact with and manage VirtMCU simulations.

### Debugging Helpers
*   **`debug/`**: GDB Python scripts for deep inspection of QEMU internal state and QOM structures.
*   **`ffi_layout_check/`**: A utility to verify that C and Rust struct layouts match, preventing memory corruption in FFI boundaries.
*   **`analyze_coverage.py`**: Analyzes guest code coverage by mapping `drcov` trace files to ELF symbols.
