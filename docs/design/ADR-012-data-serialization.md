# ADR-012: Hybrid Data Serialization Architecture

**Status:** Accepted
**Date:** 2026-04-16

## Context

The `virtmcu` simulation framework communicates with external tools (like SystemC adapters, `zenoh-clock` masters, and Firmware Studio telemetry dashboards) heavily. Originally, we used raw C `struct` types mapped directly over sockets and parsed manually in Python using `struct.unpack("<QQII", data)`. 

As the structs evolved (e.g., adding `vtime_ns` to MMIO requests), this resulted in "silent failures" where mismatched sizes or misaligned padding caused Python parsers and SystemC adapters to hang or interpret garbage data. We needed a robust, cross-language serialization format. However, the QEMU main loop (TCG thread) is hyper-sensitive to latency, meaning heavy serialization overhead (like JSON or even FlatBuffers builders) directly inside the MMIO or Clock hooks would drastically lower simulation Instructions-Per-Second (IPS).

## Decision

We chose a **Hybrid Serialization Strategy** tailored to the synchronous vs. asynchronous nature of the data paths.

### 1. Synchronous Hot Paths (MMIO & Clock)
For blocking, high-frequency bridges where every nanosecond counts, we use **Strict Versioned Binary Structs**.
*   **Format:** Raw C `struct` marked `__attribute__((packed))`.
*   **Single Source of Truth:** `hw/rust/virtmcu-api/src/lib.rs` defines all payloads (`MmioReq`, `ClockAdvanceReq`, etc.) natively in Rust.
*   **Handshake Safety:** Every connection must immediately send and verify a `VirtmcuHandshake` struct containing a `MAGIC` (0x564D4355) and an incrementing `VERSION`.
*   **Downstream (Python):** We auto-generate Python `dataclass` bindings via `scripts/gen_vproto.py`. Downstream tools import `vproto.py` to seamlessly pack/unpack data without hardcoded format strings.
*   **Downstream (C/C++):** Legacy components use `hw/misc/virtmcu_proto.h` which must be kept in sync with the Rust SSOT.

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

*   **For MMIO/Clock Control:** You **MUST** implement the 8-byte `virtmcu_handshake` upon connecting to the socket or Zenoh queryable. If using Python, simply import `vproto.py`. If using Rust, add the `virtmcu-api` crate as a dependency and use the exported packed structs.
*   **For Telemetry Consumption:** Subscribe to `sim/telemetry/trace/<node_id>`. If using Rust, consume the `virtmcu-api` crate to access the pre-generated FlatBuffers bindings rather than compiling `telemetry.fbs` manually.
