# Rust FFI Safety & Memory Audit

## Overview
This document summarizes the invariants and safety boundaries established during the migration of QEMU plugins to native Rust in `hw/rust/`.

## 1. Object State Lifecycles (The `rust_state` Pointer)
Every QEMU C-struct backed by Rust (e.g., `ZenohNetClient`, `ZenohUiQEMU`) contains a `void *rust_state` (or equivalent typed raw pointer) field.

### Invariants:
1. **Allocation:** `rust_state` is instantiated exclusively via `Box::into_raw(Box::new(State))` inside the device's `realize` or `instance_init` hook.
2. **Access:** Dereferencing `rust_state` is done via `unsafe { &*(state_ptr) }`. This is ONLY valid because the pointer is guaranteed by QEMU's object model to remain stable for the lifetime of the `Object`.
3. **Deallocation:** `rust_state` is reclaimed ONLY in `instance_finalize` or `cleanup` (for netdevs) via:
   ```rust
   if !s.rust_state.is_null() {
       unsafe { drop(Box::from_raw(s.rust_state)); }
       s.rust_state = ptr::null_mut();
   }
   ```
   **Verification:** All QOM finalize methods (`clock_instance_finalize`, `ui_instance_finalize`, `netdev_cleanup`, etc.) have been audited and explicitly check for `.is_null()` before reclaiming memory, preventing double-frees.

## 2. Big QEMU Lock (BQL) Thread Safety
QEMU is single-threaded for guest MMIO and timer delivery (the BQL). Zenoh, however, executes subscriber callbacks in a multi-threaded async pool.

### Invariants:
1. **State Mutation:** Zenoh worker threads MUST NOT mutate QEMU state directly.
2. **FFI Wrappers:** Calls to `qemu_set_irq`, `virtmcu_timer_mod`, and `qemu_chr_be_write` are wrapped in `virtmcu_bql_lock()` and `virtmcu_bql_unlock()` inside the Zenoh callback closures.
3. **Priority Queues (Performance Iteration):** To prevent Zenoh worker threads from stalling while waiting for the BQL, `netdev` was refactored to use a lock-free MPSC channel (`crossbeam_channel::unbounded`). The Zenoh thread simply pushes to the channel. The `rx_timer_cb` (which executes on the QEMU main thread) pulls from the channel into a thread-local `BinaryHeap`.

## 3. Endianness Assumptions
When memory is accessed via `MemoryRegionOps`, the Rust implementation assumes `DEVICE_LITTLE_ENDIAN`.
### Invariants:
1. Rust reads/writes 64-bit values as raw integers.
2. QEMU handles byte-swapping if the guest architecture does not match the `DEVICE_LITTLE_ENDIAN` declaration.

## Conclusion
The `unsafe` blocks within `hw/rust/` strictly encapsulate:
1. QOM raw pointer dereferencing with explicit lifecycle bounds.
2. Direct calls to exported QEMU C-functions (`qemu_send_packet`, `qemu_chr_be_write`, `qemu_set_irq`).
3. Acquisition of the Big QEMU Lock (BQL).

No unbounded slices or unverified raw buffer accesses exist outside of controlled Zenoh framing boundaries.
