# ADR 015: Enterprise Lock-Free Logging Architecture

## Context
The VirtMCU project previously relied on a primitive `vlog!` macro that directly wrapped the QEMU C function `VirtMCU_log` (which wraps `printf`). This approach violated several core enterprise mandates:
1. **Real-Time Safety:** `printf` acquires libc locks and performs blocking I/O on the host. Calling this from a vCPU MMIO handler blocks the Big QEMU Lock (BQL), causing severe virtual time jitter and potentially stalling the simulation.
2. **Context Blindness:** Developers had to manually prepend module tags (`[actuator]`) and node IDs to log strings.
3. **Missing Telemetry (VTime):** Logs lacked Virtual Time (VTime) stamps, making deterministic debugging impossible. Wall-clock time is useless in a Parallel Discrete Event Simulation (PDES).
4. **BQL Deadlock Hazard:** Reading the QEMU virtual clock (`qemu_clock_get_ns`) is only safe when the BQL is held. Attempting to log from background Zenoh threads caused undefined behavior or deadlocks when trying to fetch the time.

## Decision
We are adopting a **"Zero-Friction, Lock-Free Binary Telemetry"** architecture.

### 1. The "Invisible Context" (Global Node & Time)
Since VirtMCU strictly enforces **one simulated node per QEMU process**, the `NodeID` is effectively a process-global constant.
- We maintain a `GLOBAL_NODE_ID: AtomicU32`. It is auto-initialized by the first peripheral that reads its QOM properties.
- We maintain a `GLOBAL_VTIME: AtomicU64`. It is continuously updated by `clock` (or safely queried from QEMU if the BQL is held).

### 2. Lock-Free Bounded Queue
We use a `crossbeam_channel::bounded` queue for log events. The logging macro does **not** call `printf`. Instead, it:
1. Formats the string into a stack-allocated fixed byte array (`[u8; 256]`) to avoid `malloc` locks on the hot path.
2. Pushes the raw struct (`LogEntry`) into the channel via a lock-free `try_send`.
If the queue is full (the simulation is logging faster than the host can print), messages are silently dropped to preserve RT-safety and determinism.

### 3. Dedicated Background Drain Thread
A singleton background thread is spawned automatically on the first log. It drains the queue, formats the rich structured log line, and prints it:
`[VTime: 12345000 ns] [Node: 4] [INFO] [actuator] Actuator ready`

### 4. Semantic Macros
The old `vlog!` is deprecated. We introduce semantic macros: `sim_err!`, `sim_warn!`, `sim_info!`, `sim_debug!`, and `sim_trace!`.

## Consequences
- **Pros:** 100% Real-Time safe for vCPU threads. No manual tagging required. Guaranteed deterministic VTime ordering. BQL-safe.
- **Cons:** Extremely long log messages (>256 bytes) will be truncated. High-volume logging might drop messages if the queue fills up.
- **Rejected Alternatives:** *TLS Context* was rejected because NodeID is process-global. *Pure Binary Linker Sections (defmt)* were rejected due to the complexity of offline decoding dynamically loaded QEMU `.so` plugins.
