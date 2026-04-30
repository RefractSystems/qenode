# Chapter 6: Peripheral Subsystem

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

### `BqlGuarded<T>` vs. `Mutex<T>`
In standard Rust, shared state is protected by `std::sync::Mutex<T>`. However, because most peripheral code runs under the BQL, a `Mutex` is redundant and risky—it can lead to deadlocks if not managed carefully.

VirtMCU mandates the use of `BqlGuarded<T>` for state accessed from MMIO handlers, timers, and `SafeSubscriber` callbacks. It uses `UnsafeCell<T>` internally and debug-asserts that the BQL is held at every access point.

---

## 2. Peripheral Fidelity & Timing

VirtMCU prioritizes **Software-Observable Fidelity** over microscopic cycle-accuracy. We model the physical duration of transmissions to ensure that firmware sees realistic baud rates and bus contention.

### The Problem of Immediate Execution
In a simple emulator, writing to a UART is "instant." A CPU could blast 1,000 bytes into a UART in 1,000 virtual nanoseconds. This creates a "virtual time flood" that violates physical laws and hides real firmware bugs (like buffer overflows).

### The Solution: Event-Driven Backpressure
VirtMCU standardizes on **Event-Driven Virtual Timers** (Option C). 
1.  **Accept**: The peripheral accepts the data into a software FIFO instantly from the CPU's perspective.
2.  **Schedule**: It calculates the physical transmission delay (e.g., 86.8 µs for a UART byte) and schedules a `QEMUTimer` tied to `QEMU_CLOCK_VIRTUAL`.
3.  **Execute**: Only when the timer fires is the byte "dispatched" to the simulation bus and the `TX_EMPTY` interrupt raised.

This ensures the firmware is naturally throttled to the hardware's physical limits while maintaining the high execution speed required for CI/CD.

---

## 3. Serialization & The Wire

All data sent over the simulation bus must be deterministic and cross-platform.
- **Explicit Endianness**: Always use `.to_le_bytes()`.
- **FlatBuffers**: Use the core schema (`core.fbs`) for all inter-process messages.
- **Zero-Copy**: Telemetry and high-volume data use zero-copy FlatBuffers construction to minimize host overhead.
