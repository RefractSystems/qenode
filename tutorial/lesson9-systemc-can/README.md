# Lesson 9 — Co-simulating Shared Buses with SystemC

This lesson demonstrates how `virtmcu` can be used to co-simulate a virtual MCU in QEMU alongside a highly accurate, shared physical bus modeled in SystemC. We use a minimal "CAN-lite" implementation as our example.

## Why Co-Simulate?

QEMU is excellent at executing CPU instructions quickly and modeling standard SoC peripherals (UARTs, timers, memory controllers). However, modeling complex physical layers—such as the arbitration phases of a Controller Area Network (CAN) bus, signal integrity, or analog attenuation—is out of scope for a pure CPU emulator.

By bridging QEMU to SystemC:
1. **QEMU** runs the firmware and handles the CPU, memory, and simple MMIO peripherals.
2. **SystemC** models the complex timing, bit-level arbitration, and electrical characteristics of the shared medium.

## Architecture of the Educational CAN Model

Our example consists of three major components:

1. **`mmio-socket-bridge` (QEMU)**: An MMIO peripheral instantiated dynamically via Device Tree. Any firmware reads/writes to its address space are forwarded over a UNIX socket to the SystemC adapter.
2. **`CanController` (SystemC)**: A simple TLM-2.0 target module in `tools/systemc_adapter/main.cpp`. It exposes registers (TX_ID, TX_DATA, CMD, STATUS, RX_ID, RX_DATA) and triggers virtual IRQs back to QEMU when a frame is received.
3. **`SharedMedium` (SystemC + Zenoh)**: A module that simulates the physical CAN bus. When a `CanController` transmits, the `SharedMedium` encapsulates the frame and publishes it to the Zenoh topic `sim/systemc/frame/{node_id}/tx`.

## The Zenoh Coordinator

The `zenoh_coordinator` (developed in Phase 6 and upgraded in Phase 9) subscribes to `sim/systemc/frame/*/tx`.

When it receives a frame from Node 1:
1. It inspects the `delivery_vtime_ns` timestamp.
2. It adds a propagation/arbitration delay (e.g., 1ms).
3. It forwards the frame to `sim/systemc/frame/Node2/rx`.

When Node 2's `SharedMedium` receives the message, it queues it. The SystemC kernel waits until the virtual time reaches the delivery time, then passes the frame to Node 2's `CanController`, which finally raises an IRQ in Node 2's QEMU instance.

## Execution Flow

1. Firmware on Node 1 writes to the `TX_DATA` and `TX_ID` registers of the bridge.
2. Firmware writes `1` to the `CMD` register.
3. QEMU pauses the vCPU and sends an MMIO `WRITE` request over the UNIX socket to SystemC.
4. SystemC's `CanController` processes the write, constructs a `CanFrame`, and passes it to the `SharedMedium`.
5. `SharedMedium` publishes the frame via Zenoh-C.
6. SystemC sends a response to QEMU, unpausing the vCPU.
7. The Zenoh Coordinator routes the frame to Node 2 with a deterministic virtual time delay.
8. Node 2's SystemC adapter receives the frame via Zenoh.
9. Node 2's `SharedMedium` simulates local bus delay and gives the frame to Node 2's `CanController`.
10. Node 2's `CanController` sends an `IRQ_SET` message over the UNIX socket to Node 2's QEMU.
11. Node 2's QEMU raises the GIC interrupt, and the guest firmware jumps to the ISR to read the `RX_DATA`.

## Key Takeaway

SystemC handles the *bus*, while QEMU handles the *CPU*. Because all communication is stamped with virtual time (`delivery_vtime_ns`), the entire multi-node simulation remains perfectly deterministic and repeatable, regardless of the host OS scheduler or network latency.
