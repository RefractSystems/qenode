# virtmcu Rust Peripheral Plugins

This directory contains the Rust-based QOM (QEMU Object Model) plugins for the virtmcu project.

## Directory Structure (ARCH-19)

The crates are organized into logical subdirectories based on their role in the simulation:

- `backbone/`: Core simulation infrastructure.
  - `clock`: Virtual clock synchronization and management.
  - `mmio-socket-bridge`: Synchronous MMIO-over-socket bridge.
  - `remote-port`: Low-latency bridge for CPU-to-CPU or CPU-to-FPGA.
  - `transport-zenoh`: Zenoh-backed implementation of the `DataTransport` API.
- `comms/`: Communication peripherals and networking.
  - `netdev`: Virtual Ethernet adapter (Zenoh-backed).
  - `chardev`: Virtual character device / UART (Zenoh-backed).
  - `canfd`: Controller Area Network with Flexible Data-rate.
  - `flexray`: FlexRay deterministic networking.
  - `spi`: Serial Peripheral Interface.
  - `ieee802154`: Low-rate wireless personal area networks.
  - `wifi`: Wireless networking adapter.
- `observability/`: Simulation observability and interaction.
  - `actuator`: Consumer for actuator commands.
  - `telemetry`: Producer for simulation trace events.
  - `ui`: Graphical user interface components.
- `mcu/`: MCU-specific peripheral implementations.
  - `s32k144-lpuart`: Specialized UART for NXP S32K144.
- `common/`: Shared utilities and internal APIs.
  - `virtmcu-api`: Central wire protocol and `DataTransport` traits.
  - `virtmcu-qom`: Safe Rust wrappers for QEMU Object Model (QOM) FFI.
  - `rust-dummy`: Template for new peripheral models.

## Development Mandates

- **Binary Fidelity**: Plugins must behave exactly like the physical silicon they emulate.
- **Determinism**: No local timers or wall-clock dependencies in the hot simulation loop.
- **BQL Safety**: Always use the RAII guards from `virtmcu-qom::sync::Bql` when accessing shared state.
- **Transport Agnostic**: Use the `DataTransport` trait in `virtmcu-api` for all emulated data plane traffic. Do not hardcode Zenoh-specific logic in peripheral crates.
