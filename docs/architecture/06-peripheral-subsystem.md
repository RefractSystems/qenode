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

Because network delivery acts as a host-level bridge (running on `QEMU_CLOCK_REALTIME`), its execution is non-deterministic relative to the guest. VirtMCU enforces a **Two-Stage Delivery Pipeline** via the `DeterministicReceiver` utility to ensure bit-identical results:

1. **Stage 1 (Host Ingress)**: The `DeterministicReceiver` receives a packet from the transport, decodes the `delivery_vtime_ns`, and places it into a virtual-time-sorted priority queue. It does NOT touch guest registers or raise IRQs.
2. **Stage 2 (Virtual Time Delivery)**: A `QomTimer` (bound to `QEMU_CLOCK_VIRTUAL`) fires at exactly `delivery_vtime_ns`. It drains the queue and invokes the peripheral's delivery callback under the BQL. **This** is the only safe context for mutating guest-visible state or signaling vCPUs.

> [!MANDATE]
> **Never mutate guest-visible state or wake a suspended vCPU directly inside a transport callback.** Always route through Stage 2.

If you attempt to bypass this pipeline and write directly to state in Stage 1, the guest may see data "from the future" or experience non-deterministic execution paths based on host OS scheduling jitter.

### `virtmcu_qom::sync::Mutex<T>` vs. Atomics
In standard Rust, shared state is protected by `std::sync::Mutex<T>`. **`std::sync::Mutex<T>` is BANNED in VirtMCU peripherals** because it deadlocks with the BQL.

VirtMCU mandates the following synchronization patterns:
1. **Atomics (`AtomicBool`, `AtomicU64`)**: Use for simple flags and status registers. This is the "Gold Standard" for performance and determinism.
2. **`virtmcu_qom::sync::Mutex<T>`**: A QEMU-backed mutex compatible with the BQL. Use this for complex state that requires locking (e.g., a `VecDeque` backlog) and for guarding `QemuCond` wait loops.
3. **`BqlGuarded<T>` (DEPRECATED)**: Historically used to enforce BQL-only access. This is being phased out in favor of the `MmioDevice` trait and `DeterministicReceiver`, which handle implicit synchronization via the BQL and `DrainToken` exchanges.

> [!TIP]
> **Safety-by-Construction:** If your peripheral follows the `MmioDevice` trait and uses `DeterministicReceiver` for ingress, your state is automatically synchronized under the BQL, and you should rarely need manual Mutexes. Use Atomics for high-frequency status flags.

### Co-Simulation and BQL Discipline: `CoSimBridge`

**Architectural Mandate:** `CoSimBridge` is strictly for the QEMU boundary to non-simulation infrastructure (test runners, the coordinator itself, HiL hardware). It MUST NOT be used for peripherals that participate in the simulation graph (e.g., sensors, actuators, radios, SPI). Simulation peripherals MUST route all traffic through the `DeterministicCoordinator` using `DeterministicReceiver` to respect the PDES quantum barrier.

When a boundary peripheral needs to block waiting for an external infrastructure response (like over a Remote Port Unix socket or Chardev), it must yield the BQL to prevent main loop deadlocks. Historically, developers had to manually orchestrate a complex 4-step unlock/wait/relock sequence, which was prone to Lock-Order Inversion deadlocks and Use-After-Free bugs during teardown.

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
