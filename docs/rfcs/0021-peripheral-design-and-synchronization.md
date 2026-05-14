# RFC-0021: Unified Peripheral Design and Deterministic Synchronization

## Status
Proposed

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
| **`QomTimer`** | The "Virtual Clock." Replaces sleep. Schedules callbacks at precise virtual timestamps. |
| **`SafePublisher`** | Lock-free network egress. Prevents the vCPU from blocking on network congestion. |
| **`DeterministicReceiver`** | **Mandatory for Ingress.** Replaces the now-banned `SafeSubscription`. Automatically acquires the BQL, manages generation counters to prevent UAF, and uses `QomTimer` to sort and deliver packets at their correct Virtual Time, eliminating manual heap/timer boilerplate. |
| **`Bql::temporary_unlock()`** | An RAII guard to safely drop and re-acquire the BQL during MMIO polling loops. *(Note: RFC-0023 proposes wrapping this behavior behind explicit type-state `DrainToken` exchanges).* |
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
