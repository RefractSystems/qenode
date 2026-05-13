# Peripheral Subsystem

## Native Rust Peripherals

The VirtMCU peripheral ecosystem is built on a foundation of memory safety and high-performance concurrency. By leveraging the `virtmcu-qom` library, developers can author complex peripheral models (UARTs, CAN-FD controllers, WiFi radios) in Rust that load directly into the QEMU address space as dynamic plugins.

---

## 1. Concurrency, Safety, and the BQL

The Big QEMU Lock (BQL) is the primary synchronization mechanism in the emulator. VirtMCU enforces strict safety rules to prevent deadlocks and race conditions across the C/Rust boundary.

### Threading Model
- **VCPU Threads**: Execute guest instructions. MMIO handlers (read/write callbacks) execute in this context.
- **Main Loop Thread**: Manages QMP, GDB, and asynchronous I/O.
- **Peripheral Threads**: Peripherals may spawn background threads (e.g., Zenoh subscribers).

**Crucial Invariant**: Only ONE thread can hold the BQL at any time. MMIO handlers and QEMUTimer callbacks are invoked by QEMU with the BQL **already held**.

### The Two-Stage Delivery Pipeline

Because `SafeSubscription` acts as a host-level bridge (running on `QEMU_CLOCK_REALTIME` to prevent deadlocks when the BQL is yielded), its execution is entirely non-deterministic from the guest's perspective. It fires whenever the host OS network stack delivers a packet.

Therefore, **"Never mutate guest-visible state or wake a suspended vCPU directly inside a `SafeSubscription` callback."**

Every peripheral receiving asynchronous data MUST strictly implement a **Two-Stage Delivery Pipeline**:
1. **Stage 1 (Host Time / `SafeSubscription`)**: The callback receives the packet, decodes the `delivery_vtime_ns` from the VirtMCU FlatBuffer header, places the payload into an internal priority queue sorted by virtual time, and schedules a `QomTimer` (bound to `QEMU_CLOCK_VIRTUAL`). It does NOT touch guest registers, raise IRQs, or signal `wait_yielding_bql` condition variables.
2. **Stage 2 (Virtual Time / `QomTimer`)**: When QEMU's deterministic virtual clock naturally reaches `delivery_vtime_ns`, the timer callback fires. **This** is where the peripheral moves the data into guest-visible MMIO registers, asserts interrupts, or calls `cond.notify_all()` to wake up a polling vCPU.

If you attempt to bypass the timer and write directly to state in Stage 1, the guest may see data "from the future," or experience severe race conditions resulting in non-deterministic execution paths depending on host OS scheduling.

### `BqlGuarded<T>` vs. `Mutex<T>`
In standard Rust, shared state is protected by `std::sync::Mutex<T>`. However, because most peripheral code runs under the BQL, a `Mutex` is redundant and risky—it can lead to deadlocks if not managed carefully.

VirtMCU mandates the use of `BqlGuarded<T>` for state accessed from MMIO handlers, timers, and `SafeSubscriber` callbacks. It uses `UnsafeCell<T>` internally and debug-asserts that the BQL is held at every access point.

### Co-Simulation and BQL Discipline: `CoSimBridge`
When a peripheral needs to block waiting for an external co-simulation response (like over a Remote Port Unix socket), it must yield the BQL to prevent main loop deadlocks. Historically, developers had to manually orchestrate a complex 4-step unlock/wait/relock sequence, which was prone to Lock-Order Inversion deadlocks and Use-After-Free bugs during teardown.

VirtMCU now uses an **Inversion of Control (IoC)** pattern via the `virtmcu_qom::cosim::CoSimBridge` framework primitive.

Developers implement the `CoSimTransport` trait (providing pure socket/I/O logic) and pass it to a `CoSimBridge`. The framework automatically handles:
1. **Safe BQL Yielding**: Uses `QemuCond::wait_yielding_bql` internally, structurally guaranteeing that the BQL is yielded before blocking and re-acquired safely without Lock-Order Inversion against local mutexes.
2. **Background I/O Thread**: Spawns and manages the OS-bound socket/receive thread.
3. **RAII vCPU Teardown (`VcpuDrain`)**: Tracks active vCPUs in the MMIO path. During device teardown (in `Drop`), it automatically waits for all blocked vCPUs to drain (with a bounded timeout) before freeing the device memory, strictly avoiding Use-After-Free regressions.

To execute a blocking co-simulation request, the vCPU simply calls:
```rust
let response = self.bridge.send_and_wait(request, TIMEOUT_MS);
```

---

## 2. Strict Dependency Injection (DI)

...

---

## 3. The Engineering Standards

To ensure enterprise-grade reliability and binary fidelity, every peripheral must adhere to the following standards.

### 1. The FFI Gate (Layout Verification)
QEMU is written in C; VirtMCU peripherals are written in Rust. The boundary between them is a set of shared `struct` layouts. If these layouts drift (e.g., after a QEMU version bump), the result is a catastrophic segfault.
- **Mandatory Asserts**: All core QOM structs in Rust MUST contain `assert!` checks for `size_of` and `offset_of` within their `TypeInit`.
- **The Gate**: Before any build is promoted, `./cargo run -p virtmcu-test-runner -- lint` must be executed to verify ground truth against the QEMU binary. Use `--fix` to auto-sync Rust layouts to C.

### 2. MMIO Relative Offsets
The `mmio-socket-bridge` delivers **region-relative offsets**, not absolute guest addresses. 
- **Rule**: Peripheral models must NEVER attempt to add a base address to the received offset. 
- **Endianness**: VirtMCU standardizes on **Little Endian** for the simulation wire. `0xDEADC0DE` is sent as `DE C0 AD DE`.

### 3. Safe Peripheral Teardown
Thread-spawning peripherals are the #1 source of "Stale Process" and "Use-After-Free" bugs. Every peripheral must implement the **Canonical Shutdown Sequence**:
1.  **Set `running = false`** (while holding the state lock).
2.  **Broadcast** all condition variables to wake blocked threads.
3.  **Wait via `drain_cond`** until `active_vcpu_count == 0`. We use a "Drain Pattern" rather than bounded spinloops to avoid time-bomb UAFs.
4.  **Join** the background thread.
5.  **Drop** the `Arc<SharedState>`.

### 4. Unsafe Rust: Precise Rules
- **Packed Structs**: Always use `ptr::read_unaligned` when accessing fields of a `#[repr(packed)]` struct to avoid undefined behavior.
- **Serialization**: Use `to_le_bytes()` and `from_le_bytes()`. Never use `mem::transmute` for wire protocols.
- **NE_BYTES Ban**: `to_ne_bytes()` and `from_ne_bytes()` are strictly BANNED for any value that leaves the process.
- **FFI Scoping**: Limit `unsafe` blocks to a single FFI call. Never aggregate multiple operations into one block.

---

## 4. Peripheral Fidelity & Timing

...

---

## See Also
*   **[BQL and Concurrency](../fundamentals/10-bql-and-concurrency.md)**: The locking rules every peripheral developer must follow.
*   **[MMIO and Registers](../fundamentals/02-mmio-and-registers.md)**: The guest-facing side of these peripheral models.
*   **[The FlexRay Case Study](../postmortem/2026-05-01-flexray-rc-11-segfault.md)**: A postmortem on complex peripheral state synchronization.
