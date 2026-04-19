# virtmcu Core Hardening Analysis

## What Is the Core?

The virtmcu "core" is the set of components that every peripheral and higher-level phase depends
on. A bug here silently corrupts all work built on top of it. The core consists of:

| Layer | Crate / File | Role |
|---|---|---|
| **Protocol wire format** | `virtmcu-api/src/lib.rs` | Shared structs between Rust and Python (`ZenohFrameHeader`, `ClockAdvanceReq`, `ClockReadyResp`, `MmioReq`, `SyscMsg`) |
| **Zenoh session management** | `virtmcu-zenoh/src/lib.rs` | Opens authenticated sessions; failure here = all devices dead |
| **BQL / sync primitives** | `virtmcu-qom/src/sync.rs` | Wraps QEMU's Big QEMU Lock; misuse = deadlock or data corruption |
| **Clock synchronization** | `zenoh-clock/src/lib.rs` | Drives virtual time advancement; stall = sim hangs forever |
| **UART delivery pipeline** | `zenoh-chardev/src/lib.rs` | Ordered RX delivery; bug = firmware sees garbled or missing bytes |
| **Network delivery pipeline** | `zenoh-netdev/src/lib.rs` | Ordered Ethernet RX delivery; structurally mirrors chardev |

Everything above these layers (new SoC peripherals, actuators, telemetry, 802.15.4 radio) inherits
whatever correctness properties (or defects) live here. This document catalogs every known risk and
prescribes testable fixes.

---

## Bug Inventory

### CRITICAL — can crash or deadlock the simulator

#### C-1: `libc::abort()` on clock stall (zenoh-clock:214)

```rust
// zenoh_clock_quantum_wait_internal
if result.timed_out() && start.elapsed() > timeout {
    vlog!("[virtmcu-clock] ERROR: Timeout ...");
    unsafe { libc::abort() };  // kills the entire QEMU process
}
```

**What goes wrong**: Any transient network partition, slow time authority, or CI runner hiccup
triggers an immediate, non-recoverable process kill. The error is indistinguishable from a SIGSEGV
in the CI log.

**Fix**: Return an error code (`CLOCK_ERROR_STALL = 1`) from `zenoh_clock_quantum_wait_internal`.
The caller (`zenoh_clock_cpu_halt_cb`) re-acquires BQL and logs the stall, then lets the
simulation continue (time authority can retry). For catastrophic stalls the test harness already
has a wall-clock timeout; `abort()` provides nothing beyond that except an uglier failure.

**Test**: Unit test that drives `on_clock_query` to timeout and asserts stall code is returned
without process termination.

---

#### C-2: `unwrap()` on `Mutex::lock()` in hot paths (multiple files)

Occurrences:
- `zenoh-clock/src/lib.rs:189`, `205`, `251`, `270`, `513`
- `zenoh-chardev/src/lib.rs:246`

```rust
let mut heap = state.local_heap.lock().unwrap();  // panics if poisoned
```

**What goes wrong**: If any thread holding the mutex panics (e.g. index-out-of-bounds inside
`rx_timer_cb`), the mutex becomes poisoned. The next `lock().unwrap()` call panics from a
*different* thread — and Rust panics inside `extern "C"` FFI callbacks are undefined behavior.
QEMU does not catch them; the result is a silent stack corruption or segfault.

**Fix**: Replace `unwrap()` with `unwrap_or_else` that logs and returns a safe default, or use
`.lock().unwrap_or_else(|p| p.into_inner())` (recover from poison). For the heap mutex in
`rx_timer_cb`, recovering the poisoned inner value is safe because the heap is not logically
invalid after a panic during a peek/pop.

---

#### C-3: BQL double-unlock potential (zenoh-clock:129 + 136–142)

```rust
let was_locked = unsafe { virtmcu_qom::sync::virtmcu_bql_locked() };
if was_locked {
    unsafe { virtmcu_qom::sync::Bql::unlock() };   // manual unlock
}
// ... wait ...
if was_locked {
    let _bql = virtmcu_qom::sync::Bql::lock();     // RAII re-acquire
    std::mem::forget(_bql);                         // keep it locked — prevents Drop from unlocking
}
```

The `std::mem::forget` is intentional and *appears* correct: we want the BQL to remain locked
after this function returns, so we drop the guard without releasing. However:

- `Bql::unlock()` has **no assertion** that the BQL is actually held. If
  `virtmcu_bql_locked()` returns a stale false-negative (possible under races on some QEMU
  builds), we call `virtmcu_bql_unlock()` on an unlocked BQL — undefined behaviour in pthreads.
- The `debug_assert!` in `zenoh_clock_quantum_wait_internal` (line 180) is **only active in debug
  builds**. Release CI runs without it.

**Fix**:
1. Promote the debug assertion to a runtime check with an `eprintln!` + `return` on failure.
2. Add a compile-time note on `Bql::unlock` that the caller must have verified via
   `virtmcu_bql_locked()` immediately before calling.

---

#### C-4: Subscriber callback acquires BQL (zenoh-chardev:358, 382)

```rust
// Inside Zenoh subscriber callback (runs on a Zenoh I/O thread)
let _bql = Bql::lock();
unsafe { virtmcu_timer_mod(rx_timer, ...) }
```

**What goes wrong**: The Zenoh I/O thread blocks waiting for BQL while the QEMU main thread holds
BQL and is itself blocked waiting for a Zenoh reply (e.g. during clock sync in `on_clock_query`).
Classic ABBA deadlock.

**Conditions for the deadlock**:
1. QEMU main thread holds BQL → calls `session.get("sim/clock/advance/0")` → waits for Zenoh
2. Zenoh I/O thread processes a chardev RX sample → tries to acquire BQL → blocks
3. The Zenoh query reply is processed on the same I/O thread → cannot run → deadlock

**Fix**: Instead of acquiring BQL inside the subscriber callback, use an atomic flag + a
dedicated QEMU bottom-half (BH) or `qemu_bh_schedule()` to defer the `virtmcu_timer_mod` call
to a context where BQL is already held and Zenoh is not blocked.

As a near-term mitigation: verify that the Zenoh clock query and chardev subscriber run on
*separate* Zenoh I/O threads (they do in current Zenoh 1.x by default), and add an integration
test that performs simultaneous clock advance + chardev RX to expose any regression.

---

### HIGH — silent data corruption or resource leak

#### H-1: Unbounded channels cause OOM under sustained flood (zenoh-chardev)

```rust
let (tx, rx) = unbounded();  // RX subscriber → rx_timer_cb (MPSC)
let (tx_pub_send, tx_pub_recv) = unbounded::<Vec<u8>>();  // write → TX thread
```

**What goes wrong**: A fast sender (e.g. pre-publishing 50k frames at once) pushes packets into
the channel faster than `rx_timer_cb` drains them. The `BinaryHeap` in `local_heap` grows
without bound, consuming ~50 bytes per `OrderedPacket` (vtime u64 + Vec). At 50k packets this is
~2.5 MB — acceptable. But at 1M+ (future stress scenarios) it becomes a problem.

`zenoh-netdev` uses `bounded(1024)` which silently drops packets when full — also wrong.

**Fix**: Use `bounded(65536)` for both channels. On overflow, log the drop and set a
`rx_overflow: AtomicU64` counter that is visible via a QEMU monitor command or telemetry topic.

---

#### H-2: Global mutable statics (`GLOBAL_CLOCK`, `GLOBAL_TELEMETRY`)

```rust
static mut GLOBAL_CLOCK: *mut ZenohClock = ptr::null_mut();
```

**What goes wrong**:
1. Multiple `zenoh-clock` devices instantiated (e.g. multi-node QEMU) → last `realize()` wins;
   previous pointer leaked and hooks overwritten.
2. Hooks (`virtmcu_cpu_set_halt_hook`) are function pointers in QEMU's C code. There is no
   mutex protecting them; a concurrent `instance_finalize` + `realize` race is UB.

**Fix**: Use `once_cell::sync::OnceCell<*mut ZenohClock>` with a compare-exchange to enforce
single-instance and fail loudly on duplicate realize. For multi-node scenarios a `RwLock<Vec>`
keyed by node_id is the correct long-term solution.

---

#### H-3: Query-reply threads spawned without bound (zenoh-clock:290)

```rust
std::thread::spawn(move || {
    let _ = query.reply(query.key_expr(), resp_bytes.as_slice()).wait();
});
```

Every `ClockAdvanceReq` spawns a new OS thread. Under a fast time-authority loop (100 Hz × N
nodes) this creates N×100 threads per second. On Linux the default thread stack is 8 MB →
eventual OOM or OS thread limit.

**Fix**: Use a single background thread with a `bounded(1)` channel for replies. Since
`on_clock_query` already serializes via `quantum_done`/`quantum_ready`, there is at most one
reply in flight at a time.

---

#### H-4: Heartbeat thread leaks after QEMU device teardown (zenoh-clock:504)

```rust
std::thread::spawn(move || loop { ... std::thread::sleep(Duration::from_millis(1000)); });
```

The heartbeat thread holds a `backend_ptr` that was freed in `instance_finalize` via
`Box::from_raw`. The thread continues running after finalize and dereferences a freed pointer.

**Fix**: Use an `Arc<AtomicBool> shutdown_flag`. Set it to `true` in `instance_finalize` before
`Box::from_raw`. The heartbeat thread checks it and exits.

---

#### H-5: Legacy-header "compatibility" path silently misroutes packets (zenoh-chardev:351)

```rust
if data.len() < 12 {
    // Compatibility with legacy flood tests that don't send headers
    let packet = OrderedPacket { vtime: 0, ... };  // deliver immediately
    ...
}
```

Any truncated or malformed Zenoh message (network corruption, partial write) is silently
treated as a legacy packet and delivered at vtime=0, bypassing the priority queue entirely.
This is a silent correctness hole that makes bugs in senders look like firmware bugs.

**Fix**: Remove the legacy path. Any payload < 12 bytes is an error; log it and discard. If
legacy senders exist, fix them.

---

#### H-6: `std::mem::forget(queryable)` leaks the Zenoh queryable (zenoh-clock:498)

```rust
std::mem::forget(queryable);
```

This prevents the queryable from being deregistered when the Zenoh session is dropped. If
`zenoh_clock_init_internal` is called more than once (e.g. hot-reload of a plugin), the first
queryable leaks and continues firing `on_clock_query` with a dangling `backend_ptr`.

**Fix**: Store the queryable in `ZenohClockBackend` and drop it explicitly in
`instance_finalize`.

---

### MEDIUM — incorrect behaviour, not immediately fatal

#### M-1: `virtmcu-zenoh` blocks QEMU init thread for up to 4 seconds

```rust
for i in 0..40 {  // 40 × 100ms = 4 seconds
    let routers: Vec<_> = info.routers_zid().wait().collect();
    ...
    std::thread::sleep(Duration::from_millis(100));
}
```

QEMU's `realize()` chain is synchronous. Every device that calls `open_session()` blocks the
entire QEMU startup for up to 4s per device. With 4 devices this is 16s startup delay before the
first TB executes — observable in CI timing.

**Fix**: The router-reachability check should time out after a configurable period (default 2s,
controlled by a `VIRTMCU_ZENOH_CONNECT_TIMEOUT_MS` env var). The current 40×100ms is already 4s
— document it and make it configurable.

---

#### M-2: `on_clock_query` reads `ClockAdvanceReq` via `read_unaligned` (zenoh-clock:234)

```rust
let req = unsafe { std::ptr::read_unaligned(payload_bytes.as_ptr() as *const ClockAdvanceReq) };
```

The 16-byte `payload_bytes` slice from Zenoh may not be aligned to 8-byte boundary. The
`read_unaligned` is correct on x86 but is UB on strict-alignment architectures (ARM64 in some
configurations). Additionally, the length check `payload.len() < 16` is done earlier (line 229)
but a Zenoh impl could theoretically return a shorter `to_bytes()` slice despite the length check
passing. Using `read_unaligned` on a slice that is exactly 16 bytes is fine but deserves a
defensive length assertion immediately before the cast.

**Fix**: Use `bytemuck::from_bytes` (safe, checked) or explicitly use
`struct.unpack("<QQ", payload[:16])` pattern mirrored in Rust with
`u64::from_le_bytes(payload_bytes[0..8].try_into().unwrap())`.

---

#### M-3: `Bql::lock()` returns `BqlGuard` that double-unlocks if combined with `mem::forget`

In `zenoh_clock_cpu_halt_cb`:
```rust
let _bql = virtmcu_qom::sync::Bql::lock();
std::mem::forget(_bql);  // intentional: keep BQL locked
```

This pattern is correct *in isolation* but invisible to future developers. If someone removes
`mem::forget` thinking it is a leak, they introduce a double-unlock. The API has no way to
express "lock, transfer ownership to C caller."

**Fix**: Add a `Bql::lock_forget()` method that acquires BQL and returns nothing (or returns a
`ManuallyDrop<BqlGuard>`), making the intent explicit.

---

#### M-4: `vlog!` buffer overflows silently at 256 bytes (virtmcu-qom/src/lib.rs)

```rust
const LOG_BUF_SIZE: usize = 256;
let mut buf = [0u8; LOG_BUF_SIZE];
```

Format strings longer than 256 bytes are silently truncated. Clock contention reports, which
include floating-point percentages and counters, can exceed this easily with many nodes.

**Fix**: Use a dynamic `String` + `CString::new()` path, or increase the buffer to 1024 bytes.

---

## What Is Not Tested at Runtime

| Risk | Where | Missing Test |
|---|---|---|
| Stall error code propagation | zenoh-clock | No test for `STALL` response reaching Python caller |
| Clock sync handshake | zenoh-clock | No unit test for `quantum_done` → `quantum_ready` state machine |
| `OrderedPacket` min-heap semantics | zenoh-chardev/netdev | No Rust unit tests; Python tests cover encoding, not Rust heap |
| Poison recovery on mutex | zenoh-chardev | No test for behavior after a panic inside `rx_timer_cb` |
| Concurrent clock + chardev RX | integration | ABBA deadlock (C-4) has no regression test |
| Bounded vs unbounded channel overflow | zenoh-chardev | No test for drop-on-overflow with counter |
| Legacy short-payload path | zenoh-chardev | No test asserting discard behavior |
| Multi-instance `GLOBAL_CLOCK` | zenoh-clock | No test; would require two `-device zenoh-clock` |
| Router disconnect recovery | virtmcu-zenoh | No test for behavior when router dies mid-sim |
| Frame size = 0 in ZenohFrameHeader | virtmcu-api | Tested in Python but not Rust |
| `ClockReadyResp.error_code = STALL` | virtmcu-api | Python unpack tested; Rust struct not |
| BQL held/not-held assertion | virtmcu-qom | `debug_assert!` only; no release-mode coverage |
| Topic naming for multi-node | all chardev | No test verifying `sim/chardev/1/rx` ≠ `sim/chardev/0/rx` |

---

## Can We Reach 100% Coverage?

**Python tests**: Yes — the protocol (virtmcu-api equivalents in Python), topic naming, and wire
format are pure logic with no external dependencies. 100% is achievable and should be enforced
with `--cov-fail-under=100` for `tests/test_vproto.py` and `tests/test_phase8_uart_stress.py`.

**Rust unit tests** (`cargo test`): Currently **0%** — there are no `#[cfg(test)]` modules in
any of the core crates. The pure-logic parts can reach ~60–70%:
- `ZenohClockBackend` state machine (no QEMU FFI needed if factored out)
- `OrderedPacket` ordering
- Wire format encoding/decoding
- Topic string construction

**Rust integration tests** (with QEMU): ~40–50% of `zenoh-clock` and `zenoh-chardev` is
reachable via the existing shell-based integration tests. The remaining 50% requires mocking
QEMU FFI (`virtmcu_bql_lock`, `qemu_clock_get_ns`, etc.) — feasible with `#[cfg(test)]` stub
modules.

**Realistic target**: 90%+ Python, 60%+ Rust (pure logic), 40%+ Rust (FFI-dependent paths via
integration).

---

## Hardening Roadmap

### Tier 1 — Fix before next SoC peripheral (prevents cascading bugs)

1. **Replace `abort()` with stall error code** in `zenoh_clock_quantum_wait_internal`
2. **Replace all `unwrap()` on Mutex::lock()** with poison-recovery
3. **Add `Arc<AtomicBool>` shutdown flag** to heartbeat thread
4. **Store queryable in struct** instead of `mem::forget`; drop on finalize
5. **Remove legacy short-payload path** in zenoh-chardev subscriber; replace with logged discard
6. **Add Rust unit tests** for `OrderedPacket` ordering, wire format, topic naming

### Tier 2 — Structural safety (next sprint)

7. **Replace `GLOBAL_CLOCK` static mut** with `OnceLock<AtomicPtr>`; fail on duplicate realize
8. **Replace per-query thread spawn** with a single reply thread + bounded channel
9. **Switch chardev/netdev RX channels** to `bounded(65536)` with overflow counter
10. **Add `Bql::lock_forget()`** to make BQL transfer-to-C explicit
11. **Add `VIRTMCU_ZENOH_CONNECT_TIMEOUT_MS` env override** for CI speed tuning

### Tier 3 — Coverage infrastructure

12. **Add `cargo llvm-cov` target** to Makefile (pure Rust logic, no QEMU)
13. **Add `--cov-fail-under=90`** for Python test suite
14. **Add integration test** for simultaneous clock-advance + chardev RX (ABBA deadlock canary)
15. **Add integration test** for stall-code propagation via `sim/clock/advance/{id}` reply

---

## Tools to Incorporate

| Tool | Purpose | How |
|---|---|---|
| `cargo llvm-cov` | Rust line/branch coverage | `cargo llvm-cov --workspace --html` in Makefile |
| `loom` | Rust concurrency model checker | Test `quantum_ready`/`quantum_done` state machine |
| `miri` | UB detector for Rust | Run on pure-Rust units (not FFI-heavy code) |
| `ThreadSanitizer` | C data race detector | Add `-fsanitize=thread` to QEMU build in CI |
| `AddressSanitizer` | Heap use-after-free | Add to QEMU + plugin build for debug CI tier |
| `pytest --cov --cov-fail-under` | Python coverage gate | Already have pytest-cov; add threshold |
| `proptest` / `quickcheck` | Property-based Rust tests | Fuzz `ZenohFrameHeader` encode/decode |
| `cargo-deny` | Dep audit | Block unmaintained / RUSTSEC-flagged crates |
