# RFC-0018: Safe Peripheral BQL Yielding

## Status

Accepted. **Partially superseded:**
- `CoSimBridge` (RFC-0027) now owns BQL yielding and `VcpuDrain` for boundary peripherals (UART bridges, HiL, remote-port).
- `#[qom_device]` (RFC-0023) replaced the `#[derive(MmioDevice)]` macro proposed here — peripheral authors use the `Peripheral` trait directly.
- `MmioResult::wait_for(...)` remains authoritative for simulation peripheral MMIO handlers.
- Direct calls to `Bql::temporary_unlock()` from peripheral code are BANNED per `CLAUDE.md`; the framework wraps them.
- `BqlContext` (RFC-0041) provides compile-time proof that BQL is held in MMIO handlers and timer callbacks.

## Context

VirtMCU ensures fidelity by running unmodified firmware written for bare-metal silicon. Bare-metal firmware frequently uses tight polling loops (e.g., `while(!REG_STATUS_READY);`) rather than `WFI` to check peripheral state.

In a QEMU-based hypervisor, executing an MMIO read acquires the Big QEMU Lock (BQL). A vCPU that tight-polls an MMIO register rapidly locks and unlocks the BQL without yielding to the host OS scheduler. This aggressive contention starves the QEMU main loop thread, preventing it from acquiring the BQL to process incoming messages — producing a simulation livelock.

The initial mitigation required developers to manually insert `Bql::temporary_unlock()` and `std::thread::yield_now()` into `unsafe extern "C"` MMIO callbacks. This was fragile: a missed yield silently reintroduced the starvation bug, and `yield_now()` is a spin-yield that may immediately re-contend for the lock.

## Decision

We eliminate manual BQL management in peripheral development via two mechanisms:

### 1. True Blocking (`wait_yielding_bql`)

Instead of `yield_now()`, peripherals waiting on asynchronous data suspend the vCPU using QEMU condition variables (`QemuCond::wait_yielding_bql`). This removes the vCPU from the host OS run queue until the peripheral's background thread signals the condvar — eliminating CPU waste and guaranteeing deterministic wakeup latency.

### 2. The `MmioResult::wait_for` Closure Pattern

Peripheral MMIO handlers do not manage the yield loop. They declare a condition and a result value; the framework owns the yield/wait loop:

```rust
// Current Peripheral trait (RFC-0041 updated signatures):
impl Peripheral for MyState {
    fn read(&self, offset: u64, _size: u32, ctx: &BqlContext) -> MmioResult<'_> {
        match offset {
            REG_STATUS => MmioResult::wait_for(
                || self.data.get(ctx).has_data,   // condition — framework polls this
                || STATUS_RX_PENDING,              // value returned when ready
            ),
            _ => MmioResult::Ready(0),
        }
    }
}
```

The `ctx: &BqlContext` parameter is the compile-time proof (RFC-0041) that this code executes with BQL held. The developer declares intent; the framework handles synchronization.

### 3. `#[qom_device]` Macro

The `#[qom_device]` macro (RFC-0023) generates the `MemoryRegionOps`, FFI shims, and BQL dispatch, consuming the `Peripheral` trait implementation. Peripheral authors write no `extern "C"` callbacks.

## Drawbacks

- Increased complexity in `virtmcu-qom` to handle closure lifetimes across the `wait_yielding_bql` boundary.
- The yield loop is invisible to the peripheral author, which can make debugging MMIO stalls harder without framework-level tracing.

## Rationale and Alternatives

**Alternative: Manual `yield_now()` per peripheral**: the prior art. Produced BQL starvation bugs when developers forgot to call it. Rejected.

**Alternative: `std::thread::sleep` in MMIO handlers**: banned — halts the simulation and breaks Virtual Time. Rejected.

## Related

- RFC-0006: Binary Fidelity
- RFC-0023: Safe QOM Macros (`#[qom_device]` replaces `#[derive(MmioDevice)]`)
- RFC-0027: CoSimBridge RAII Framework (supersedes this RFC for boundary peripherals)
- RFC-0041: Safe QOM Framework Boundaries (`BqlContext` token; `ctx` in MMIO handler signatures)
