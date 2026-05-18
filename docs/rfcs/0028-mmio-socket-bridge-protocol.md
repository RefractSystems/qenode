# RFC-0028: MMIO Socket Bridge Architecture

## Status
Accepted

## Context
To facilitate out-of-process integration testing and native co-simulation, VirtMCU requires a mechanism to forward MMIO read/write operations from the QEMU guest directly to an external process. While Zenoh is used for distributed multi-node communication, it is too heavy for cycle-accurate, blocking MMIO interception.

## Decision
We implement the `mmio-socket-bridge`, a dedicated QOM device that maps a configurable memory region. Any guest memory access within this region is serialized over a native Unix Domain Socket (UDS) to an external adapter process.

## Reference-level explanation
- **Memory Mapping**: The device is mapped via the Device Tree (DTB) dynamically at startup using `yaml2qemu`.
- **Protocol**: A lightweight, blocking Request/Response protocol over UDS. A read operation sends a request and blocks the vCPU (yielding the BQL via `CoSimBridge`) until the external process responds with the value.
- **SVD Handshake**: The bridge performs an initial handshake with the external client, sending an SVD hash to guarantee the external process agrees on the memory layout and peripheral definitions.

## Consequences
- **Positive**: Enables writing integration tests completely natively in Rust (using `virtmcu-test-runner`) without compiling test code to an ARM ELF.
- **Negative**: High latency compared to in-process memory access. BQL yielding during socket wait can impact overall simulation speed if polled aggressively.