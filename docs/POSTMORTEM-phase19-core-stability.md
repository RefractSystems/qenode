# Postmortem: Phase 19.1 — Core Stability, Endianness & Protocol Convergence

**Date:** 2026-04-19  
**Component:** `hw/rust/virtmcu-qom`, `hw/rust/zenoh-actuator`, `hw/rust/zenoh-clock`, `hw/misc/virtmcu-rust-ffi.[ch]`  
**Severity:** High — intermittent hangs, incorrect register reads, and protocol desyncs  
**Status:** Resolved

---

## Background

As we transitioned from Phase 18 (Native Migration) to Phase 19 (Multi-Node Stability), several "ghost" bugs emerged. These were bugs that only manifested under specific timing conditions or when interacting with external test scripts. The most visible symptoms were:
1. Rust peripherals returning incorrect data for 32-bit register reads.
2. Zenoh actuator integration tests failing to parse payloads.
3. The simulator hanging or "overshooting" virtual time boundaries during heavy I/O.

---

## Bug 1 — The Cross-Language Enum Trap (Endianness)

### Symptom

A Rust-based peripheral implemented a simple ID register. A read from the guest returned `0x00000000` or a byte-swapped value, even though the Rust code was clearly returning `0xDEADBEEF`.

### Root Cause

In QEMU, the `MemoryRegionOps` struct requires an `endianness` field. The Rust bindings (`virtmcu-qom`) used a hardcoded constant:

```rust
// hw/rust/virtmcu-qom/src/memory.rs (OLD)
pub const DEVICE_LITTLE_ENDIAN: i32 = 1; 
```

However, checking `include/exec/memop.h` in the QEMU source revealed:

```c
enum device_endian {
    DEVICE_NATIVE_ENDIAN,
    DEVICE_BIG_ENDIAN,       // 1
    DEVICE_LITTLE_ENDIAN,    // 2
};
```

The Rust constant was off-by-one. It was telling QEMU that the Rust peripheral was `DEVICE_BIG_ENDIAN`. QEMU was then byte-swapping the "Big Endian" output from Rust to "Little Endian" for the ARM guest, corrupting the data.

### Fix

Corrected the constant to match the C enum:

```rust
pub const DEVICE_LITTLE_ENDIAN: i32 = 2;
```

**Lesson:** **Never trust your memory for constants defined in an external system.** When writing FFI bindings or re-implementing C enums in Rust, always perform a "Sanity Check Turn": grep the header file in the actual dependency (`third_party/qemu`) to confirm the literal value. Enums in C are just integers, and their values depend on declaration order.

---

## Bug 2 — The "Invisible" Protocol Change (Actuator Payloads)

### Symptom

Actuator integration tests (`test/actuator/`) failed with "Incomplete payload" errors. The test script expected 12 bytes but received only 4.

### Root Cause

To support deterministic replay and telemetry, we had standardized that all Zenoh I/O payloads must be prefixed with an 8-byte `vtime_ns` (virtual timestamp). 

The Python test scripts were updated to expect this prefix:
```python
# test_actuator.py
vtime, value = struct.unpack("<QI", payload)
```

However, the `zenoh-actuator` Rust implementation was still sending only the raw 4-byte `value`.

### Fix

Modified the Rust sender to pack the virtual time (retrieved via FFI) before the value:

```rust
let mut payload = Vec::with_capacity(12);
payload.extend_from_slice(&vtime_ns.to_le_bytes());
payload.extend_from_slice(&value.to_le_bytes());
backend.session.put(key, payload).wait();
```

**Lesson:** **Protocols are contracts.** In a distributed simulation, a change in the "wire format" requires a synchronized update across all producers (C/Rust models) and consumers (Python tests/telemetry tools). If you change a serialization format, use `grep_search` to find every site that calls `put` or `get` on that topic.

---

## Bug 3 — The WFI "Stall" (Clock Slaving)

### Symptom

In `slaved-suspend` mode, if the guest CPU executed a `WFI` (Wait For Interrupt) instruction, the simulation would hang. Zenoh would send a clock advance request, but QEMU would never reply.

### Root Cause

When a CPU is halted (idle), QEMU's TCG engine stops executing instructions. In standard QEMU, time "stops" or advances via timers. In our slaved mode, we hooked the TCG loop to block on Zenoh. But if the CPU is halted, it **never enters the TCG loop**.

The Zenoh thread was waiting for the vCPU thread to hit a boundary, but the vCPU thread was asleep in the "halted" state, waiting for an interrupt that would never come because time wasn't moving.

### Fix: The 4-Flag Handshake & Offset Jumping

1. **Protocol Upgrade:** Moved from a 3-flag to a 4-flag handshake (`quantum_ready`, `quantum_done`, `delta_ns`, `vtime_ns`) protected by a Mutex and Condvar to ensure the Zenoh thread can wake the vCPU even if it's halted.
2. **Clock Jumping:** When the guest is idle, we must manually "jump" the virtual clock forward to the next Zenoh boundary. We updated `virtmcu_icount_advance` to modify `cpu_clock_offset` in non-icount modes:

```c
// hw/misc/virtmcu-rust-ffi.c
void virtmcu_icount_advance(int64_t delta_ns) {
    if (icount_enabled()) {
        // ... icount logic ...
    } else {
        // Suspend mode: manually push the offset
        atomic_add(&timers_state.cpu_clock_offset, delta_ns);
    }
}
```

**Lesson:** **Simulation time must move even when the guest is doing nothing.** "Idle" in the guest is not "Idle" in the simulator. The simulator must still participate in the global clock handshake to allow other nodes (or physics engines) to progress.

---

## Debugging Methodology: The "Divergence Search"

When debugging these issues, I used a technique I call **Divergence Search**:

1. **Isolate the Boundary:** I ran the simulation with `VIRTMCU_LOG=debug` and compared the logs of a "working" Phase (e.g., Phase 7) with the "broken" Phase 19.
2. **Trace the Constant:** To find the endianness bug, I traced the memory access path: `Guest LD` -> `TCG` -> `MemoryRegionOps.read`. I noticed that the `size` and `addr` were correct, but the *returned value* was being mangled *after* the Rust function returned. This pointed directly to the `MemoryRegion` configuration, not the logic inside.
3. **The "Stall" Detection:** I used `strace -p <pid> -e futex` to see which thread was stuck. I saw the Zenoh thread stuck in `pthread_cond_timedwait` and the vCPU thread stuck in `ppoll` (the QEMU main loop idle). This "split" confirmed the vCPU was idling while Zenoh was waiting—a classic deadlock of expectations.

---

## Tips for Developers Stuck in Similar Bugs

1. **Verify your Constants:** If you are working across C and Rust (or any FFI), **manually verify literals**. Enums are the #1 source of "silent" corruption because the code compiles perfectly but the logic is shifted.
2. **Log the Wire:** When inter-process communication (Zenoh, Sockets) fails, don't trust your code's `println!`. Use a "sniffer" (like `zenoh-cli` or a custom script) to see the **exact bytes** on the wire.
3. **The BQL is your Shadow:** If QEMU hangs, 90% of the time it's the Big QEMU Lock (BQL). If you are blocking on an external event (Zenoh query, Socket read), you **must** release the BQL before blocking, or the IO thread will never be able to wake you up.
4. **Assume "Works on My Machine" is a Lie:** If tests pass in isolation but fail in the suite, look for "stale state." Are there leftover Zenoh routers? Is there a shared UNIX socket file? Use `make clean-sim` religiously.

---

## Timeline

| Step | Discovery | Fix |
|------|-----------|-----|
| 1 | Phase 19 integration tests show byte-swapped register values. | Identified `DEVICE_LITTLE_ENDIAN` mismatch; corrected Rust constant. |
| 2 | Actuator tests fail with "Incomplete payload". | Traced Python test expectations; updated Rust `zenoh-actuator` to include `vtime_ns`. |
| 3 | Simulator hangs during guest idle (`WFI`). | Implemented 4-flag handshake in `zenoh-clock` and `cpu_clock_offset` jumping. |
| 4 | Final Verification. | Ran `make test-all` (110 units, 19 phases). All Green. |
