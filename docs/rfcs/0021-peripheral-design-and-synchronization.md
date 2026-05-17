# RFC-0021: Unified Peripheral Design and Deterministic Synchronization

## Status
Accepted (referenced as the "Gold Standard" mandate in `CLAUDE.md`/`AGENTS.md`)

> Note: The utility table below predates RFC-0025. `SafePublisher` has been
> replaced by the zero-copy `DataTransport::reserve()/commit()` API in all
> peripheral code. Treat the row below as historical; the canonical egress
> API is defined in RFC-0025.

## Context
VirtMCU is a deterministic multi-node simulation framework. To achieve bit-identical results across runs while maintaining high execution speed, peripheral models must bridge the gap between the asynchronous host environment (network I/O, OS threads) and the synchronous guest environment (Virtual Time, CPU instructions).

Unmodified firmware often relies on timing-sensitive patterns (polling, interrupt latency) that are easily broken by asynchronous host behavior. This record formalizes the design patterns required for all VirtMCU peripherals to ensure safety, determinism, and developer productivity.

## Research & Alternatives (SOTA)
We analyzed several State-of-the-Art (SOTA) simulation architectures to identify the optimal balance for VirtMCU:

| Framework | Synchronization Approach | Pros | Cons |
| :--- | :--- | :--- | :--- |
| **SystemC** | Cooperative Coroutines | Cycle-accurate; no data races. | Strictly single-threaded; slow for complex systems. |
| **gem5** | Global Event Queue | Extreme micro-architectural detail. | Very slow; steep learning curve. |
| **Renode** | Quantum-Based Barriers | Inherently parallel; easy to write. | Slower than QEMU due to lack of optimized JIT. |
| **VirtMCU (QEMU)** | **JIT + BQL Yielding** | **Unmatched speed**; runs real binaries. | Requires complex BQL management for safety. |

**Conclusion:** VirtMCU adopts the "Yield-on-Read / Async-on-Write" pattern. This allows us to keep QEMU's high-speed execution engine while fixing its legacy synchronization flaws.

## Decision: The Peripheral Design Mandate
All peripherals in the VirtMCU ecosystem must adhere to the following architectural constraints and utilize the provided framework utilities.

### 1. Design Constraints

*   **Transport Agnosticism:** Peripherals MUST NOT depend on specific transport implementations (e.g., Zenoh, Unix Sockets). They must use the `DataTransport` trait injected via dependency injection (DI).
*   **Prohibition of Blocking/Sleeping:** Calling `std::thread::sleep` or blocking on network sockets is BANNED. It halts the entire simulation and breaks Virtual Time. Use `QomTimer` instead.
*   **Cooperative BQL Yielding:** Any MMIO read handler that returns a "not ready" status to a polling guest MUST yield the Big QEMU Lock (BQL) to prevent simulation deadlocks.
*   **Deterministic VTime Delivery:** Incoming network data must be timestamped and delivered via a `QomTimer` scheduled for the packet's specific virtual delivery time.
*   **State Encapsulation:** All peripheral state must live in a dedicated Rust struct. To ensure safe boundary crossings and prevent memory leaks, developers MUST use the declarative `#[qom_device]` macro and the unified `Peripheral` trait (as formalized in RFC-0023), replacing manual C-FFI `MmioDevice` implementations.

### 2. Provided Utilities & Hooks

To simplify development, the framework provides the following "Golden Path" utilities:

| Utility | Purpose |
| :--- | :--- |
| **`ClosureTimer`** | **Preferred for peripheral-owned timers.** Accepts an `FnMut(&BqlContext)` closure — no `extern "C"` callbacks, no raw pointer casting, correct drop ordering enforced by the framework. The `&BqlContext` passed to the closure enables compile-time proof that BQL is held (RFC-0041). Use `QomTimer` only inside `virtmcu-qom` framework code. |
| **`DataTransport::reserve()/commit()`** | **Mandatory for Egress.** Zero-copy publication API (RFC-0025). Replaces the deprecated `SafePublisher` and `transport.publish(...)`. No `encode_frame` boilerplate in peripheral code. |
| **`VtimeIngress`** | **Mandatory for Ingress.** Replaces the now-banned `SafeSubscription`. Automatically acquires the BQL, manages generation counters to prevent UAF, and uses `QomTimer` to sort and deliver packets at their correct Virtual Time, eliminating manual heap/timer boilerplate. |
| **`MmioResult::wait_for(...)`** | **Mandatory for blocking MMIO reads.** Framework-owned condvar wait. Replaces direct `Bql::temporary_unlock()` calls in peripheral code (RFC-0018 / RFC-0023). |
| **`CoSimBridge`** | **For co-simulation bridges only** (`mmio-socket-bridge`, `remote-port`). Owns vCPU registration, BQL yielding, and RAII teardown. See RFC-0027. |
| **`VcpuDrain`** | Ensures safe peripheral destruction by waiting for all active MMIO calls to finish. Inherently managed by the `Peripheral` trait wrapper (RFC-0023) to prevent developers from forgetting the lock guard. |

## Consequences
*   **Positive:** Guaranteed bit-identical determinism for multi-node simulations.
*   **Positive:** Unified developer experience across sensors, actuators, and radio models.
*   **Positive:** High simulation throughput by leveraging QEMU's JIT while maintaining host-level safety.
*   **Negative:** Developers must learn the VirtMCU-specific synchronization primitives rather than using standard Rust/C++ threading tools.

## Related
- RFC-0006: Binary Fidelity
- RFC-0011: Zenoh Federation Bus
- RFC-0018: Safe Peripheral BQL Yielding
- RFC-0023: Safe QOM Macros and Boilerplate Eradication
- RFC-0041: Safe QOM Framework Boundaries via Type-State (`BqlContext`, `ClosureTimer`, `dynamic_cast_qom`)
