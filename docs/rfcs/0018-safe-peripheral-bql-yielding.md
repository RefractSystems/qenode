# RFC-0018: Safe Peripheral BQL Yielding via MmioDevice Trait

## Status
Accepted

## Context
VirtMCU ensures fidelity by running unmodified firmware that was originally written for bare-metal silicon. Bare-metal firmware frequently utilizes tight polling loops (e.g., `while(!REG_STATUS_READY);`) rather than `WFI` (Wait For Interrupt) to check peripheral states. 

In a QEMU-based hypervisor, executing an MMIO read acquires the Big QEMU Lock (BQL). If a guest vCPU tight-polls an MMIO register, it rapidly locks and unlocks the BQL without yielding to the host OS scheduler. This aggressive contention starves the QEMU main loop thread, preventing it from acquiring the BQL to process incoming network packets (e.g., Zenoh messages), leading to a system-wide simulation deadlock (livelock).

Our initial mitigation involved developers manually inserting `Bql::temporary_unlock()` and `std::thread::yield_now()` into the `unsafe extern "C"` MMIO callbacks. However, this is suboptimal:
1. **Busy-Yielding:** `yield_now()` is a spin-yield. The vCPU thread re-enters the OS scheduler and may immediately wake up to re-contend for the lock, wasting CPU cycles and relying on scheduler luck.
2. **Fragility & Boilerplate:** Peripheral developers must manually write unsafe C-FFI callbacks and remember to suppress linter errors (`// virtmcu-allow: yield`). A missed yield silently re-introduces the starvation bug.

## Decision
We will eliminate manual BQL management in peripheral development by transitioning to a structurally safe, closure-based polling pattern enforced via a new `MmioDevice` trait and proc-macro.

### 1. True Blocking (`wait_yielding_bql`)
Instead of `yield_now()`, peripherals waiting on asynchronous data must suspend the vCPU using QEMU condition variables (`QemuCond::wait_yielding_bql`). This explicitly removes the vCPU from the host OS run queue until the peripheral's Zenoh background thread signals the condvar, eliminating CPU waste and guaranteeing deterministic wakeup latency.

### 2. The `wait_for` Closure Pattern
To make the starvation bug unrepresentable, peripheral logic will no longer return primitive integers or use simple enums for status. Instead, they will use a framework-owned closure pattern:

```rust
fn read_status(state: &State, offset: u64) -> MmioResult {
    match offset {
        REG_STATUS => MmioResult::wait_for(
            || state.has_data,        // condition — framework polls this
            || STATUS_RX_PENDING,     // value when ready
        ),
        _ => MmioResult::Ready(0),
    }
}
```
The framework takes ownership of the yield/wait loop. The developer declares the condition and the value, completely removing BQL knowledge from the peripheral business logic.

### 3. `#[derive(MmioDevice)]` Macro
All `unsafe extern "C"` MMIO boilerplate across all peripherals (sensor, radio, actuator, etc.) will be eliminated. A procedural macro will generate the `MemoryRegionOps`, the FFI shims, and the BQL dispatch logic, consuming the safe `MmioDevice` trait implementation.

## Consequences
- **Positive:** BQL starvation bugs become impossible to express in peripheral code.
- **Positive:** Zero CPU waste during bare-metal polling loops, improving simulation scalability.
- **Positive:** Peripheral development becomes entirely safe Rust, free of FFI and lock-management boilerplate.
- **Negative:** Increased complexity in the `virtmcu-qom` framework to handle closure lifetimes across the `wait_yielding_bql` boundary.

## Related
- RFC-0006: Binary Fidelity
- `virtmcu-test-runner` lints (`banned_patterns.rs`)