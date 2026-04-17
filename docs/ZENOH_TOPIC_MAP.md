# virtmcu Zenoh Topic Map

This document defines the standardized Zenoh topics (KeyExpressions) used across the `virtmcu` federation. All inter-process communication in the simulation environment uses these exact channels.

**Note:** In virtmcu, the term `node_id` is typically an integer or string identifier assigned to each simulated cyber node (QEMU instance) or physics entity.

## Standard Channels

| Component | Topic / KeyExpression | Role | Payload Format |
| :--- | :--- | :--- | :--- |
| **Clock Sync** | `sim/clock/advance/{node_id}` | Queryable | `ClockAdvancePayload` (16 bytes) |
| **Telemetry** | `sim/telemetry/trace/{node_id}` | Publisher | FlatBuffers (`telemetry.fbs`) |
| **Ethernet** | `sim/eth/frame/{node_id}/tx` <br> `sim/eth/frame/{node_id}/rx` | Pub/Sub | `ZenohFrameHeader` + Ethernet Frame |
| **UART (Serial)** | `virtmcu/uart/{node_id}/tx` <br> `virtmcu/uart/{node_id}/rx` | Pub/Sub | `ZenohFrameHeader` + Raw Bytes |
| **UI LEDs** | `sim/ui/{node_id}/led/{led_id}` | Publisher | State change payload |
| **UI Buttons** | `sim/ui/{node_id}/button/{btn_id}` | Subscriber | State change payload |
| **802.15.4 Radio** | `sim/rf/802154/{node_id}/tx` <br> `sim/rf/802154/{node_id}/rx` | Pub/Sub | `ZenohFrameHeader` + Radio Frame |
| **Bluetooth HCI** | `sim/rf/hci/{node_id}/tx` <br> `sim/rf/hci/{node_id}/rx` | Pub/Sub | `ZenohFrameHeader` + HCI Frame |
| **Sensors** | `sim/sensor/{node_id}/{name}` | Subscriber | Physics data (from `mujoco_bridge`) |
| **Actuators** | `sim/actuator/{node_id}/{name}` | Publisher | Control data (to `mujoco_bridge`) |

## System / Coordinator Channels

The `zenoh_coordinator` tool acts as a network switch/L2 bridge and environment manager.

| Component | Topic / KeyExpression | Role | Payload Format |
| :--- | :--- | :--- | :--- |
| **Network Ctrl** | `sim/network/control` | Subscriber | JSON (link-quality matrices, drop probabilities) |
| **SystemC CAN** | `sim/systemc/frame/{node_id}/tx` <br> `sim/systemc/frame/{node_id}/rx` | Pub/Sub | CAN Frame |

## Protocol & Payload Details

### Synchronous Data (Clock)
The `sim/clock/advance/{node_id}` topic uses synchronous `GET` requests (Queryables). The QEMU TCG thread blocks on this until a reply is returned by the Time Authority. The payload is a packed C struct (`ClockAdvancePayload`), mapped natively in `virtmcu_proto.h`. See [ADR-012](ADR-012-data-serialization.md).

### Deterministic Sub-system (eth, uart, rf)
All cyber-world network traffic (`sim/eth/frame`, `sim/rf/`, `virtmcu/uart/`) must preserve perfect virtual time determinism. To accomplish this, all transmitted bytes are prepended with a 12-byte header:

```rust
#[repr(C)]
struct ZenohFrameHeader {
    delivery_vtime_ns: u64, // Virtual time at which the frame was sent
    size: u32,              // Number of bytes following the header
}
```

The receiving node (via `zenoh-netdev` or `zenoh-chardev`) buffers incoming payloads in an internal priority queue sorted by `delivery_vtime_ns`, and only injects the frame into the guest firmware when `QEMU_CLOCK_VIRTUAL` matches or exceeds this timestamp.

### UART Channel Defaults
If the `-chardev` topic option is unspecified, QEMU's `zenoh-chardev` backend defaults to `virtmcu/uart/{node_id}/tx|rx` instead of `sim/`.

## External References
* [ADR-011: Zenoh as the Federation Message Bus](ADR-011-zenoh-federation-bus.md)
* [ADR-012: Data Serialization](ADR-012-data-serialization.md)
* [TIMING_MODEL.md](TIMING_MODEL.md)
