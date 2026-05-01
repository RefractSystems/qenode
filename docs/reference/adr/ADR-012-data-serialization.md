# ADR-012: Hybrid Data Serialization Architecture

**Status:** Accepted
**Date:** 2026-04-16

## Context

The `VirtMCU` simulation framework communicates with external tools (like SystemC adapters, `clock` masters, and Firmware Studio telemetry dashboards) heavily. Originally, we used raw C `struct` types mapped directly over sockets and parsed manually in Python using `struct.unpack("<QQII", data)`. 

As the structs evolved (e.g., adding `vtime_ns` to MMIO requests), this resulted in "silent failures" where mismatched sizes or misaligned padding caused Python parsers and SystemC adapters to hang or interpret garbage data. We needed a robust, cross-language serialization format. However, the QEMU main loop (TCG thread) is hyper-sensitive to latency, meaning heavy serialization overhead (like JSON or even FlatBuffers builders) directly inside the MMIO or Clock hooks would drastically lower simulation Instructions-Per-Second (IPS).

## Decision

We chose a **Hybrid Serialization Strategy** tailored to the synchronous vs. asynchronous nature of the data paths.

### 1. Synchronous Hot Paths (MMIO & Clock)
For blocking, high-frequency bridges where every nanosecond counts, we use **FlatBuffers Structs** (Strictly defined, fixed-size binary layouts).
*   **Format**: FlatBuffers `struct` types within `core.fbs`.
*   **Single Source of Truth**: `hw/rust/common/virtmcu-api/src/core.fbs` defines all payloads (`MmioReq`, `ClockAdvanceReq`, etc.).
*   **Handshake Safety**: Every connection must immediately send and verify a `VirtMCUHandshake` struct containing a `MAGIC` (0x564D4355) and an incrementing `VERSION`.
*   **Downstream (Python)**: We use **FlatBuffers** Python bindings with a manual `@dataclass` wrapper (`tools/vproto.py`).
*   **Downstream (C/C++)**: Legacy components use `hw/misc/virtmcu_proto.h` which is manually aligned with the FlatBuffers layout.

### 2. Asynchronous Telemetry
For high-volume, fire-and-forget telemetry events, we use **FlatBuffers**.
*   **Format:** Defined in `hw/misc/telemetry.fbs`.
*   **Mechanism:** The QEMU TCG thread places lightweight raw structs into a concurrent queue. A dedicated background thread (`telemetry-pub`) pops these and builds the FlatBuffer using `flatcc`, zero-copying the result onto the Zenoh bus.
*   **Downstream (Firmware Studio, Dashboards):** Consumers use the generated bindings for their language of choice (Rust, Python, TS) and read directly from the memory buffer. This provides schema evolution (adding fields without breaking old clients).

## Consequences

*   **Performance Maintained:** The TCG thread remains unblocked and performs zero allocations.
*   **Safety:** Version mismatches in MMIO/Clock fail immediately and loudly during the handshake, preventing ghost bugs.
*   **Maintainability:** Updating a struct now requires incrementing the `VERSION` in `virtmcu_proto.h` and regenerating `vproto.py`. Telemetry updates are managed safely through FlatBuffers schema evolution.
*   **Dependencies:** Introduced `flatcc` (the C FlatBuffers compiler) to the QEMU build pipeline, adding a minor build step.

## Note for Downstream Consumers (e.g., Firmware Studio)

*   **For MMIO/Clock Control:** You **MUST** implement the 8-byte `VirtMCU_handshake` upon connecting to the socket or Zenoh queryable. If using Python, simply import `vproto.py`. If using Rust, add the `virtmcu-api` crate as a dependency and use the exported packed structs.
*   **For Telemetry Consumption:** Subscribe to `sim/telemetry/trace/<node_id>`. If using Rust, consume the `virtmcu-api` crate to access the pre-generated FlatBuffers bindings rather than compiling `telemetry.fbs` manually.
