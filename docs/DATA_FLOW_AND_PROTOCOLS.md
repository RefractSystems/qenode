# VirtMCU Data Flow & Protocols

VirtMCU enables multi-node, deterministic firmware co-simulation. To achieve binary fidelity and cycle-accurate execution, it uses **FlatBuffers** for strictly defined communication protocols over a Zenoh pub/sub network.

## 1. The IDL Source of Truth (`core.fbs`)

All critical simulation pathways are defined in `hw/rust/common/virtmcu-api/src/core.fbs`. The FlatBuffers compiler (`flatc`) automatically generates native libraries for both Rust (`core_generated.rs`) and Python (`tools/virtmcu_api/`), ensuring that all byte serialization, endianness formatting, and alignment constraints are universally enforced.

The core payloads include:
- `ClockAdvanceReq` / `ClockReadyResp`: Handles deterministic Quantum/Barrier synchronization.
- `MmioReq`: Tunnels ARM SysBus memory read/writes from QEMU vCPUs to Rust plugins.
- `SyscMsg`: Injects IRQs natively back into the QEMU GIC.
- `ZenohFrameHeader`: A universal 24-byte prefix encapsulating delivery time, sequencing, and length for all emulated networking traffic (UART, Ethernet, SPI).

## 2. Example Data Flow: Node A -> Node B (UART)

To understand the architecture, here is the lifecycle of a single byte transmitted from firmware running on QEMU Node A to firmware running on Node B:

1. **Firmware Execution (Node A):** The firmware natively executes a `STR` assembly instruction, writing `0x41` ('A') to the PL011 UART Data Register at memory address `0x4000_0000`.
2. **QEMU Memory Interception:** QEMU intercepts the write via the standard SysBus memory region handler. The `mmio-socket-bridge` captures this event, suspends the vCPU, and constructs a FlatBuffer `MmioReq` with `addr=0x4000_0000` and `data=0x41`.
3. **Peripheral Modeling:** The `MmioReq` is routed over a local Unix domain socket to the out-of-process Rust peripheral model (`s32k144-lpuart`). 
4. **Transport Encapsulation:** The UART model identifies it as a transmit event and calls `virtmcu_api::DataTransport::publish`. The Zenoh abstraction wraps the raw byte `0x41` by prefixing it with a `ZenohFrameHeader` (24 bytes), stamping it with the exact nanosecond virtual time (`vtime`).
5. **Network Layer:** The message is published to the Zenoh fabric.
6. **Deterministic Tie-Breaking:** The `deterministic_coordinator` subscribes to all traffic. It buffers the incoming frame until the current simulation quantum is complete, sorts all cross-node packets strictly by `delivery_vtime_ns` and `sequence_number`, and then forwards them to Node B.
7. **Reception & Decapsulation (Node B):** The `transport-zenoh` layer in Node B receives the packet, validates `len(packet) >= ZENOH_FRAME_HEADER_SIZE`, and calls `ZenohFrameHeader::unpack_slice()`. 
8. **IRQ Injection:** Node B's UART model receives the `0x41` byte. The UART model evaluates its FIFO state, realizes the RX interrupt threshold is met, and generates a `SyscMsg(type=IRQ_SET)`.
9. **Guest Notification:** The `mmio-socket-bridge` on Node B reads the `SyscMsg`, acquires the BQL (Big QEMU Lock), and triggers the ARM Generic Interrupt Controller (GIC).
10. **Firmware Execution (Node B):** Node B's firmware jumps to its native UART IRQ handler to read the byte.

By standardizing every step of this pipeline on FlatBuffers, VirtMCU ensures high-performance, layout-safe data translation bridging native C (QEMU), Rust (Peripherals), and Python (Orchestration).
