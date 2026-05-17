# RFC-0027: CoSimBridge RAII IoC Framework

## Status
Accepted

## Summary

This RFC introduces `CoSimBridge` — an Inversion of Control (IoC) container inside `virtmcu-qom` that centralizes all QEMU threading, Big QEMU Lock (BQL) yielding, and RAII teardown logic for boundary peripherals that talk to external processes without virtual-time semantics.

## Motivation

VirtMCU has two structurally different peripheral categories:

1. **Simulation peripherals** (sensors, actuators, SPI, radios): participate in the PDES quantum barrier. All traffic flows through `DeterministicCoordinator`. BQL yielding and teardown are handled by `VtimeIngress` + `VcpuDrain`.

2. **Boundary peripherals** (UART test runner bridges, hardware-in-the-loop, the coordinator's own MMIO interface): talk to external processes that have no concept of virtual time. They block on OS I/O and must yield the BQL while doing so.

Before `CoSimBridge`, each boundary peripheral (e.g., `mmio-socket-bridge`, `remote-port`) implemented its own vCPU registration, BQL yield loop, and drain-on-destroy logic. Three concrete bugs arose from this duplication:

1. **Use-After-Free on teardown**: one bridge forgot to wait for the blocking I/O thread to exit before `dlclose` freed the code segment. The thread later resumed into deallocated memory.
2. **Stale drain count**: a bridge registered a vCPU with QEMU's drain but called `VcpuDrain::acquire` on the wrong drain instance (copied rather than shared via `Arc`), so `wait_for_drain` returned prematurely.
3. **BQL hold across blocking I/O**: an early bridge called a blocking socket `recv` while holding the BQL, freezing the entire simulation until the external process responded.

All three bugs are prevented by construction when the bridge delegates to `CoSimBridge`.

## Decision

We introduce `CoSimBridge<T: CoSimTransport>` and the `CoSimTransport` trait inside `virtmcu-qom`. `CoSimBridge` owns vCPU registration, BQL yielding via `Bql::temporary_unlock()`, and RAII teardown of both the transport and the associated threads.

**Simulation vs. Boundary boundary (enforced by CLAUDE.md):** `CoSimBridge` is exclusively for boundary peripherals. Simulation peripherals MUST use `VtimeIngress::new_for_link(link_id, …)` + `reserve_link(link_id)/commit()` (RFC-0042). Using `CoSimBridge` for simulation traffic bypasses the PDES quantum barrier — no compile error prevents this, so the distinction is enforced by CLAUDE.md and code review.

## Detailed Design

### `CoSimTransport` Trait

Implementers define how to read/write data from their specific external channel:

```rust
pub trait CoSimTransport: Send + 'static {
    /// Blocking read. Called with BQL **not** held.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    /// Write. Called with BQL **not** held.
    fn write(&mut self, buf: &[u8]) -> Result<(), TransportError>;
}
```

### `CoSimBridge<T>`

Takes ownership of a `CoSimTransport`. Exposes a safe API to the peripheral's MMIO handlers. Under the hood:

- Spawns a background I/O thread that owns the transport exclusively.
- When the MMIO handler calls `bridge.read_blocking(buf)`, the bridge drops the BQL via `Bql::temporary_unlock()`, blocks on a `QemuCond`, and reacquires BQL once data arrives.
- `Drop` signals the I/O thread to exit and calls `VcpuDrain::wait_for_drain()` before the destructor returns.

## Drawbacks

- **Mandatory trait conformance**: bridge authors must implement `CoSimTransport` even for simple protocols. The trait is small, but it is still indirection.
- **Hidden BQL release**: the BQL release inside `CoSimBridge` is invisible at the call site. A developer reading `bridge.read_blocking(...)` cannot tell from the call that the BQL is dropped momentarily. The `_blocking` suffix in the method name is the only signal.
- **Not suitable for simulation traffic**: the abstraction makes it easy to route simulation traffic through a bridge by accident. The compile-time distinction between `CoSimBridge` and `VtimeIngress` is convention, not type enforcement.

## Alternatives

- **Manual per-bridge BQL management**: each bridge implements its own `Bql::temporary_unlock()` + condvar loop. This was the prior art; it produced the three bugs described in Motivation. Rejected.
- **Explicit `init()` / `deinit()` lifecycle**: require bridge authors to call cleanup functions. History shows these are forgotten. RAII via `Drop` is strictly safer. Rejected.
- **Unified abstraction for simulation and boundary peripherals**: a single type that handles both. Rejected — the PDES barrier semantics for simulation peripherals are fundamentally incompatible with blocking I/O semantics for boundary peripherals. Two types with a clear naming convention is correct.

## Prior Art

- QEMU's own `ChardevBackend` uses a similar pattern: the chardev layer owns the I/O thread, exposes event callbacks, and handles BQL coordination internally.

## Unresolved Questions

None.

## Related

- RFC-0018: Safe Peripheral BQL Yielding (the underlying `MmioResult::wait_for` and `Bql::temporary_unlock` primitives)
- RFC-0021: Unified Peripheral Design (defines the Simulation vs. Boundary boundary)
- RFC-0028: MMIO Socket Bridge Architecture (first consumer of `CoSimBridge`)
- RFC-0029: Remote Port SystemC Integration (second consumer of `CoSimBridge`)
