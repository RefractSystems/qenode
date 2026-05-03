# ADR-900: Core Hardening Roadmap

## Status
**Active / In-Progress**

## Context

The VirtMCU "core" encompasses the foundational components upon which all peripheral models and co-simulation logic are built. Defects at this layer — particularly those involving virtual time synchronization, the Big QEMU Lock (BQL), or high-frequency serialization — manifest as silent data corruption or non-deterministic hangs.

This document serves as a living inventory of identified architectural risks and a roadmap for systematic hardening.

---

## 1. Accomplishments (Tier 1 Completed)

We have successfully mitigated the most immediate "CRITICAL" failure modes:

*   **Elimination of `libc::abort()`**: The clock device no longer kills the QEMU process on transient stalls. It now propagates a `CLOCK_ERROR_STALL` code, allowing the orchestrator to decide on a retry or a graceful teardown.
*   **Panic-Safe Mutexes**: Replaced `lock().unwrap()` with poison-recovery patterns across `clock`, `chardev`, and `netdev`. This prevents "silent" FFI-boundary corruption if a background thread panics.
*   **BQL RAII Transition**: Migrated peripherals to `BqlGuarded<T>` and RAII guards, reducing the risk of manual BQL state-tracking errors.
*   **Symbol Visibility (synchronization)**: Implemented `VirtMCU_export!` and CI linter checks for `#[no_mangle]` to prevent silent `dlopen` failures.
*   **Zenoh Session Isolation**: Enforced client-mode isolation for Zenoh sessions in parallel tests, eliminating cross-talk races between pytest workers.
*   **Deterministic Routing Sync**: Implemented `ensure_session_routing` to guarantee router-side propagation of declarations before emulation start, preventing "passes locally, fails in CI" races.
*   **Coordinator Lookahead**: Enhanced the `DeterministicCoordinator` with arbitrary lookahead, allowing high-performance nodes to pre-calculate future quanta.

---

## 2. Pending Risks (The Inventory)

### HIGH — Data Corruption or Resource Leaks

#### H-1: Unbounded Channel Flooding
*   **Risk**: `chardev` and `netdev` use channels that could grow without bound if a publisher floods the bus faster than the vCPU can drain.
*   **Resolution**: Implement `bounded(65536)` with explicit overflow counters visible via telemetry.

#### H-2: Global Instance Singletons
*   **Risk**: `GLOBAL_CLOCK` and `GLOBAL_TELEMETRY` use `static mut` without enforcement of single-instance realize.
*   **Resolution**: Migrate to `OnceLock<AtomicPtr>` or a thread-safe instance registry.

#### H-3: Thread Leakage on Finalization
*   **Risk**: Background heartbeat and Zenoh subscriber threads may leak or dereference freed pointers if a device is hot-unplugged or finalized.
*   **Resolution**: Implement `Arc<AtomicBool>` shutdown signals and ensure all threads check for termination.

### MEDIUM — Performance & Correctness

#### M-1: Startup Blocking
*   **Risk**: Zenoh session initialization can block the QEMU main thread for up to 4 seconds during router discovery.
*   **Resolution**: Implement configurable `VIRTMCU_ZENOH_CONNECT_TIMEOUT_MS`.

#### M-2: Serialization Alignment
*   **Risk**: Manual `read_unaligned` or raw casts for `ClockAdvanceReq` can be risky on strict-alignment architectures.
*   **Resolution**: Fully transition all core I/O to the `vproto` (FlatBuffers) accessor patterns.

---

## 3. The Hardening Strategy

Hardening is enforced through a multi-layered defense:

1.  **Static Analysis**: `make lint` enforces bans on raw `Mutex<T>`, `thread::sleep`, and `asyncio.sleep` (without annotations).
2.  **Binary Auditing**: `verify-exports.py` ensures FFI symbols are visible.
3.  **Chaos Integration**: Jitter profiles deliberately stress the coordinator barrier to expose timing races.
4.  **Property-Based Testing**: Use `proptest` to fuzz FlatBuffers decoders for malicious or malformed packets.

## 4. Verification Requirements

No core component is considered "Hardened" until it satisfies:
- 100% Python protocol coverage.
- 60%+ Rust unit test coverage (pure logic).
- Successful completion of the `arch8_stress` suite under AddressSanitizer (ASan).
