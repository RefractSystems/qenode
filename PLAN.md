# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation. The software MUST be at the highest Enteprise Quality following the SOTA of software development.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

---

## 1. General Guidelines & Mandates

### Phase Lifecycle
Once a Phase is completed and verified, it MUST be moved from `PLAN.md` to the `/docs/COMPLETED_PHASES.md` file to maintain a clean roadmap and a clear historical record.

### Educational Content (Tutorials)
For every completed phase, a corresponding tutorial lesson MUST be added in `/tutorial`.
- **Target**: CS graduate students and engineers.
- **Style**: Explain terminology, provide reproducible code, and teach practical debugging skills.

### Regression Testing
For every completed phase, an automated integration test MUST be added to `tests/` or `test/`.
- **Bifurcated Testing**:
  - **White-Box (Rust)**: Use `cargo test` for internal state, memory layouts, and protocol parsing.
  - **Black-Box (Python)**: Use `pytest` for multi-process orchestration (QEMU + Zenoh + TimeAuthority).
  - **Thin CI Wrappers (Bash)**: Bash scripts should only be 2-3 lines calling `pytest` or `cargo test`.

### Production Engineering Mandates
- **Environment Agnosticism**: No hardcoded paths. Use `tmp_path` for artifacts.
- **Explicit Constants**: No magic numbers. Use descriptive `const` variables.
- **The Beyonce Rule**: Prove bugs with a failing test before fixing.
- **Lint Gate**: `make lint` must pass before every commit (ruff, version checks, cargo clippy -D warnings).

---

## 2. Open Items — Ordered by Priority

> **Last updated**: 2026-04-27 (audit of `close_P0s` branch, commit `74f13df`).
> **Mandatory before every commit**: `make lint && make test-unit` must both pass.
> Completed P0 history is in `docs/COMPLETED_PHASES.md`.

### Execution Order (Fundamentals Before Features)

**Determinism migration (new — highest correctness priority):**
1. **DET-1** — Fix `SafeSubscriber` bounded spinloop (P0 safety; no deps). ✅ Completed.
2. **DET-2** — Shared Zenoh session pool (after DET-1). ✅ Completed.
3. **DET-3** — `ClockSyncTransport` trait (after DET-1, DET-2). ✅ Completed.
4. **DET-4** — `UnixSocketClockTransport` (after DET-3). ✅ Completed.
5. **DET-5** — `DeterministicCoordinator` quantum barrier (parallel with DET-1..4).
6. **DET-6** — Topology-first YAML loading (after DET-5).
7. **DET-7** — Deterministic CSMA/CA seeding (after DET-6 + Phase 29.4).
8. **DET-8** — Unified PCAP logging (after DET-5).
9. **DET-9** — Wireshark extcap plugin (after DET-8; lowest priority).

**Hardware / infrastructure (existing, continue in parallel with DET work):**
10. **Phase 29** — Peripheral Time Fidelity (FIFO/timer modeling, UART backpressure).
11. **Phase 27** — FlexRay IRQs + Bosch E-Ray Message RAM.
12. **Phase 21 / 22** — WiFi / Thread Protocol expansion.
13. **Phase 30.9 + 30.9.1** — Rust systemc-adapter + stress-adapter. Needs: Phase 30.6 ✅.
14. **Phase 30.8 + 30.10** — Firmware coverage (drcov) + unified reporting.
15. **P12** — Deterministic Deadlock Detection (virtual-time budgets). Needs: P10 Part 2.1 done ✅.

---

### Completed P0 Serial Work (Tasks A–G) — Summary

| Task | Scope | Status |
|---|---|---|
| A — P02 | `remote-port`: replace unaligned cast reads with `ptr::read_unaligned` | ✅ CLOSED |
| B — P05 | Consolidate dual locking to pure Rust `Mutex`/`Condvar` in both bridges | ✅ CLOSED |
| C — P06 | Fix teardown UAF: `VcpuCountGuard` RAII + `drain_cond.wait_timeout(30s)` | ✅ CLOSED |
| D — P07 | Replace `thread::sleep` reconnect loop with `Condvar.wait_timeout` | ✅ CLOSED |
| E — P-SERIAL | Eliminate `transmute` and raw memory views; explicit `pack()`/`unpack()` | ✅ CLOSED |
| F — P-SCHEMA | Auto-generate `tools/vproto.py` from Rust source; `gen_vproto.py --check` in lint | ✅ CLOSED |
| G1 | Replace `slice::from_raw_parts` on RP packet sends with `pack_be()` | ✅ CLOSED |
| G2 | `VcpuCountGuard` RAII added to both bridges to fix panic-safety | ✅ CLOSED |
| H | `BqlGuarded<T>` migration for all Zenoh peripherals + Mutex lint | ✅ CLOSED |
| I | Fix Docker Bake tag replacement behavior (prevent manifest failures) | ✅ CLOSED |

**Audit findings fixed on top of G (2026-04-25)**:
- `bridge_write` in `remote-port` used `to_ne_bytes()` (implicit LE-host assumption) → fixed to `to_le_bytes()`.
- Read-back in `send_req_and_wait_internal` used raw `ptr::copy_nonoverlapping` into `&mut u64` → fixed to `u64::from_le_bytes()`.
- `zenoh-spi` used raw `ptr::copy_nonoverlapping` for header serialization → fixed to `ZenohSPIHeader::pack()`.
- `BqlGuarded<T>` introduced in `virtmcu-qom` to eliminate redundant `Mutex` usage in BQL-held contexts.
- `Mutex<T>` banned in `zenoh-*` peripherals via `make lint` gate (except validated background threads).
- Byte-exact `pack_be()` tests added for `RpPktBusaccess` and `RpPktInterrupt`.
- Leftover Gemini one-shot patch scripts deleted from repo root.

#### GEMINI TASK E — [P-SERIAL] (**COMPLETED ✅ 2026-04-24**)

#### GEMINI TASK F — [P-SCHEMA] (**COMPLETED ✅ 2026-04-24**)

**Why this is P0**: `tools/vproto.py` carries a false header claiming it is `AUTO-GENERATED BY proto_gen.py` from `hw/misc/virtmcu_proto.h`. Both `proto_gen.py` and `virtmcu_proto.h` are gone — relics of the pre-Rust era. The file is now hand-edited while claiming to be generated. This is a latent production bug: any field added to a Rust struct that is not also added to `vproto.py` will cause Python test servers to silently misparse all subsequent fields (off-by-N struct reads). Phase 12 test failures have already been attributed to exactly this class of drift.

**Required deliverable**: `scripts/gen_vproto.py` — a Python script that:
1. Reads `hw/rust/virtmcu-api/src/lib.rs`.
2. Parses struct definitions (field names, types, order) for `VirtmcuHandshake`, `MmioReq`, `SyscMsg`, `ClockAdvanceReq`, `ClockReadyResp`, `ZenohFrameHeader`, `ZenohSPIHeader`.
3. Maps Rust primitive types → Python struct format characters (same mapping already in `vproto.py`: `u8→B`, `u16→H`, `u32→I`, `u64→Q`, `bool→?`).
4. Emits `tools/vproto.py` — identical in structure to the current file (same `@dataclass`, same `pack()`/`unpack()` methods, same constants) but generated deterministically.
5. Updates the file header to: `# AUTO-GENERATED BY scripts/gen_vproto.py from hw/rust/virtmcu-api/src/lib.rs — DO NOT EDIT DIRECTLY.`

**CI enforcement** (add to `make lint`):
```bash
# Check vproto.py is in sync with Rust source
python3 scripts/gen_vproto.py --check
# --check mode: generates to a temp file and diffs against tools/vproto.py
# exits non-zero if any difference is found, printing the diff
```

---

### **[P01-REMAINING] ASan Boot-Time Stall: Integration Test + chardev Fix** ✅
- **Write the integration test**: `tests/test_phase7.py` verified that first quantum survives longer delays while subsequent quantums stall strictly.
- **Fix `tests/test_chardev_bql_stress.py`**: hardcoded `stall-timeout` removed; now uses environment-scaled defaults.
- **Result**: "Slow boot / fast execute" invariant proven.


---

### **[P09-REMAINING] Eliminate `#![allow(clippy::all)]` in 5 Peripheral Crates** ✅
- Fixed `zenoh-flexray`, `zenoh-802154`, `zenoh-actuator`, `zenoh-canfd`, and `zenoh-netdev`.
- Removed broad `clippy::all`, `unused_variables`, and `dead_code` suppressors.
- Fixed unused imports, variables (prefixed with `_`), collapsible matches, and unnecessary casts.
- Verified with `make lint-rust`.

---

### **[P10-Part 2.1] Zenoh Discovery via Liveliness API** ✅
- Replaced polling loop and `asyncio.sleep` in `tests/conftest.py` with deterministic Zenoh Liveliness API.
- Router (`tests/zenoh_router_persistent.py`) now declares a liveliness token `sim/router/check`.
- `wait_for_zenoh_discovery` uses an async subscriber and `liveliness().get()` for zero-polling discovery.

---

### **[R19] Fatal Security Audit** ✅
- Makefile updated to make `cargo audit` and `cargo deny` failures (or missing tools) fatal (`exit 1`) instead of warnings.

---

### **[Hardware] Phase 20.5 — SPI Bus & Peripherals** ✅
- **20.5.1** SSI/SPI Safe Rust Bindings in `virtmcu-qom` ✅.
- **20.5.2** Verified PL022 (PrimeCell) SPI controller end-to-end in `arm-generic-fdt` via SPI loopback ✅.
- **20.5.3** `hw/rust/zenoh-spi` bridge implemented ✅.
- **20.5.4** Verified SPI Loopback/Echo Firmware verification (`tests/test_phase20_5.py` is green) ✅.


### [Hardware] Phase 27 — FlexRay (Automotive) 🚧
*Depends on: Phase 5 (Bridge) ✅, Phase 19 (Rust QOM) ✅*
- [ ] **27.1.1** Add FlexRay Interrupts (IRQ lines).
- [ ] **27.1.2** Implement Bosch E-Ray Message RAM Partitioning.
- [ ] **27.2.1** Fix SystemC build regression (CMake 4.3.1 compatibility).

### [Hardware] Phase 21 — WiFi (802.11) 🚧
*Depends on: Phase 20.5 (SPI)*
- [ ] **21.7.1** Harden `arm-generic-fdt` Bus Assignment (Child node auto-discovery).
- [ ] **21.7.2** Formalize `virtmcu-wifi` Rust QOM Proxy.
- [ ] **21.2** Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### [Hardware] Phase 22 — Thread Protocol 🚧
*Depends on: Phase 20.5 (SPI), Phase 21 (WiFi)*
- [ ] **22.1** Deterministic Multi-Node UART Bus Bridge.
- [ ] **22.2** SPI 802.15.4 Co-Processor (e.g., AT86RF233).

### [Hardware] Phase 29 — Peripheral Time Fidelity & Backpressure 🚧
*Depends on: Core synchronization (Phase 18) ✅*
*Goal: Implement Software-Observable Fidelity — throttle MMIO execution to physical baud rates using `QEMUTimer`.*
- [x] **29.1** **FIFO & Timer Baseline**: Add TX/RX FIFO drain modeling using `QEMUTimer` to `rust-dummy` template, including correct reset/teardown cancellation.
- [x] **29.2** **UART Backpressure**: Upgrade `zenoh-chardev` and `s32k144-lpuart` to throttle TX interrupts at configured baud rates.
- [x] **29.3** **RX Propagation Modeling**: Queue incoming Zenoh frames and use timers to simulate reception delay before asserting RX IRQs.
- [ ] **29.4** **Radio Delays (802.15.4)**: Implement CSMA/CA backoff timers and packet air-time modeling.
- [x] **29.5** **Lifecycle Assertions**: Test that `virtmcu_timer_del` is called on peripheral disable/reset.

### [Infrastructure] Phase 30 — Deep Oxidization & Testing Overhaul 🚧
*Ongoing*
- [x] **30.6** Migrate `remote-port` to Rust (`hw/rust/remote-port/src/lib.rs`, 1096 lines).
- [ ] **30.8** Comprehensive Firmware Coverage (drcov integration).
- [ ] **30.9** Migrate `tools/systemc_adapter/` to Rust (`tools/rust/systemc-adapter/`).
  - Rewrite `main.cpp` (662 lines) + `remote_port_adapter.cpp` (96 lines) as a Rust binary sharing `virtmcu-api` types directly.
  - Add smoke test: Rust adapter ↔ `mmio-socket-bridge` round-trip MMIO read.
  - Deprecate C++ once Rust adapter passes Phase 5 stress test.
- [ ] **30.9.1** Migrate `test/phase5/stress_adapter.cpp` to Rust (depends on 30.9).
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).

### [Connectivity] Partial Implementations (verify in CI)
- **Phase 24 (CAN FD)**: `tests/test_phase24_canfd.py` — plugin-load smoke test only. Full Bosch M_CAN emulation not yet done.
- **Phase 25 (LIN)**: Listed as complete in `docs/COMPLETED_PHASES.md`; regression tests in `tests/test_phase25_*.py` should be confirmed green in CI.

### [Future] Connectivity Expansion
- [ ] **Phase 23**: Bluetooth (nRF52840 RADIO emulation).
- [ ] **Phase 26**: Automotive Ethernet (100BASE-T1).
- [ ] **Phase 28**: Full Digital Twin (Multi-Medium Coordination).

---

## 3. Migration Roadmap — Determinism, Transport Abstraction & Coordinator Barrier

> **Purpose**: This section is the definitive implementation guide for elevating virtmcu's
> multi-node determinism guarantee, abstracting the clock-sync transport, and promoting the
> coordinator to a PDES barrier. Each phase is self-contained with precise file lists,
> implementation steps, test requirements, and a binary definition of "done". Phases are
> ordered by risk and dependency. **Do not skip ahead.**
>
> **Intended audience**: Less-powerful AI coding agents and junior engineers. Every step
> is explicit. Do not infer intent — follow the steps exactly.
>
> **Prerequisite before starting any DET phase**: `make lint && make test-unit` MUST pass.

---

### **[DET-1] Fix `SafeSubscriber` Bounded-Spinloop Teardown** — P0 Safety

**Status**: 🟢 Completed.

**Status**: ✅ Completed.

**Goal**: The `Drop` implementation of `SafeSubscriber` in
`hw/rust/virtmcu-zenoh/src/lib.rs` exits after 1000 `yield_now()` calls regardless of
whether `active_count` has reached zero. When the bound is exhausted while a callback is
still running, `Drop` returns and QEMU frees peripheral state that the callback is still
reading — a use-after-free. Replace with a `Condvar`-based unconditional drain.

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs`

**Step-by-step implementation**:

1. Add `drain_cond: Arc<(Mutex<()>, Condvar)>` as a new field to `SafeSubscriber`:
   ```rust
   pub struct SafeSubscriber {
       subscriber: Option<Subscriber<()>>,
       is_valid: Arc<AtomicBool>,
       active_count: Arc<AtomicUsize>,
       drain_cond: Arc<(Mutex<()>, Condvar)>,  // NEW
   }
   ```

2. In `SafeSubscriber::new`, create the condvar and clone it into the callback:
   ```rust
   let drain_cond = Arc::new((Mutex::new(()), Condvar::new()));
   let drain_cond_cb = Arc::clone(&drain_cond);
   ```

3. Inside the subscriber callback, after `active_clone.fetch_sub(1, Ordering::SeqCst)`,
   add a notification:
   ```rust
   let (lock, cvar) = drain_cond_cb.as_ref();
   let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
   cvar.notify_all();
   ```

4. In `Drop::drop`, **remove** the bounded loop entirely:
   ```rust
   // REMOVE THIS:
   let mut attempts = 0;
   while self.active_count.load(Ordering::SeqCst) > 0 && attempts < 1000 {
       std::thread::yield_now();
       attempts += 1;
   }
   ```
   **Replace with**:
   ```rust
   let (lock, cvar) = self.drain_cond.as_ref();
   let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
   while self.active_count.load(Ordering::SeqCst) > 0 {
       guard = cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
   }
   ```

5. Add `drain_cond: Arc::clone(&drain_cond)` to the `SafeSubscriber` struct initializer.

**Unit tests** (add to the `#[cfg(test)]` block in `hw/rust/virtmcu-zenoh/src/lib.rs`):

- `test_safe_subscriber_drain_completes_under_load`: Create a `SafeSubscriber` whose
  callback sleeps for 50ms (SLEEP_EXCEPTION: test-only wall-clock drain verification).
  Drop the subscriber and assert that Drop returns only after the counter reaches zero.
  Use an `Arc<AtomicUsize>` decremented at the end of the callback; assert it is 0
  immediately after the `drop(sub)` call returns.

- `test_safe_subscriber_no_bounded_loop`: This is a compilation check. Grep for
  `attempts < ` inside `SafeSubscriber`'s `drop` body using a doc-test or comment
  assertion. (Alternatively: in CI, add `grep -n "attempts <" hw/rust/virtmcu-zenoh/src/lib.rs`
  to `make lint` and assert it finds zero matches.)

**Stress test** (add to `hw/rust/virtmcu-zenoh/src/lib.rs`):

- `stress_safe_subscriber_concurrent_drop`: In a loop of 500 iterations: create a
  `SafeSubscriber`, spawn 8 threads each publishing 20 messages to its topic, immediately
  drop the subscriber, assert `active_count == 0`. Run under `cargo test --release`.
  Zero panics or UAF reports required.

**Definition of Done**:
- [ ] The bounded loop (`attempts < 1000`) is completely absent from the `Drop` impl.
- [ ] `cargo miri test -p virtmcu-zenoh -- safe_subscriber` passes with zero errors.
- [ ] Stress test runs 500 iterations with zero failures under `cargo test --release`.
- [ ] `make lint` passes (no new `#[allow]` suppressors added).

---

### **[DET-2] Shared Zenoh Session Pool** — Performance / Code Quality

**Status**: 🟢 Completed.

**Goal**: Replace N independent `open_session()` calls across N plugins with a single
process-wide `Arc<Session>`. Currently every plugin that loads into QEMU creates its own
Zenoh executor thread pool and router TCP connection, wasting threads and startup time.

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs` — add `get_or_init_session` + `OnceLock`
- Every `hw/rust/zenoh-*/src/lib.rs` — replace `open_session` call with `get_or_init_session`

**Implementation**:

1. Add to `hw/rust/virtmcu-zenoh/src/lib.rs`:
   ```rust
   use std::sync::OnceLock;
   static GLOBAL_SESSION: OnceLock<Arc<Session>> = OnceLock::new();

   /// Returns the process-wide Zenoh session, initializing it on first call.
   /// The `router` parameter is used only on first call; subsequent calls ignore it.
   pub fn get_or_init_session(router: *const c_char) -> Result<Arc<Session>, zenoh::Error> {
       if let Some(s) = GLOBAL_SESSION.get() {
           return Ok(Arc::clone(s));
       }
       // SAFETY: open_session guarantees router is a valid C string or null.
       let session = Arc::new(unsafe { open_session(router)? });
       // OnceLock::set returns Err if already set (race); in that case, use the winner.
       let _ = GLOBAL_SESSION.set(Arc::clone(&session));
       Ok(Arc::clone(GLOBAL_SESSION.get().unwrap()))
   }

   #[cfg(test)]
   pub fn reset_global_session_for_test() {
       // Cannot reset OnceLock; tests that need isolation must use open_session directly.
   }
   ```

2. In each `hw/rust/zenoh-*/src/lib.rs`, find the call to `virtmcu_zenoh::open_session(router)`
   and change it to `virtmcu_zenoh::get_or_init_session(router)`.
   Change the struct field type from `session: Session` to `session: Arc<Session>`.
   (All method calls on `Session` work identically on `Arc<Session>` via `Deref`.)

3. `open_session` remains `pub` for tests that need an isolated session.

**Files to mechanically update** (find all with `grep -rl "open_session" hw/rust/`):
Expected: `zenoh-clock`, `zenoh-chardev`, `zenoh-netdev`, `zenoh-canfd`, `zenoh-flexray`,
`zenoh-802154`, `zenoh-actuator`, `zenoh-spi`, `zenoh-telemetry`, `zenoh-ui`.

**Unit tests**:
- `test_get_or_init_session_idempotent`: call `get_or_init_session(null)` twice; assert
  `Arc::ptr_eq` returns true (both calls return the same underlying `Session`).
- `test_session_zid_stable`: call twice; assert both `zid()` values are equal.

**Integration test** (`tests/test_det2_shared_session.py`):
- Boot QEMU with `zenoh-clock`, `zenoh-chardev`, and `zenoh-netdev` all active.
- Parse stderr for `[virtmcu-zenoh] Session returned from zenoh::open.wait()`.
- Assert this line appears exactly **once** (not three times).

**Definition of Done**:
- [ ] `grep -r "open_session" hw/rust/zenoh-*/` finds zero results (only `virtmcu-zenoh/` uses it).
- [ ] `cargo test -p virtmcu-zenoh` passes.
- [ ] Python integration test asserts single session log line.
- [ ] `make lint` passes.

---

### **[DET-3] `ClockSyncTransport` Trait** — Architecture

**Status**: 🟢 Completed.

**Goal**: Extract the clock-advance request/reply interface into a Rust trait, decoupling
the vCPU-side waiting logic in `zenoh-clock` from Zenoh types. This enables a mock for
unit tests (no Zenoh session needed) and a Unix socket implementation (DET-4).

**Files to create**:
- `hw/rust/virtmcu-zenoh/src/clock_transport.rs`

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs` — re-export `ClockSyncTransport`, `ClockAdvanceReq`,
  `ClockReadyResp`, `ClockTransportError`, `MockClockTransport` (test only)
- `hw/rust/zenoh-clock/src/lib.rs` — refactor `ZenohClockBackend` to hold a
  `Box<dyn ClockSyncTransport>` instead of Zenoh-specific fields

**Trait definition** (put in `clock_transport.rs`):
```rust
use core::time::Duration;
use virtmcu_api::{ClockAdvanceReq, ClockReadyResp};

/// Transport for the clock-advance request/reply RPC.
///
/// Implementers: [`ZenohClockTransport`], [`UnixSocketClockTransport`],
/// [`MockClockTransport`] (test only).
///
/// # Invariants
/// - `recv_advance` and `send_ready` MUST NOT be called while the BQL is held.
/// - `shutdown` is idempotent and safe to call from any thread.
pub trait ClockSyncTransport: Send + Sync + 'static {
    /// Block until a clock-advance request arrives or `timeout` elapses.
    /// Returns `None` on timeout or shutdown.
    fn recv_advance(&self, timeout: Duration) -> Option<ClockAdvanceReq>;

    /// Send the completion reply. Returns `Err` if the channel is broken.
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), ClockTransportError>;

    /// Signal shutdown; unblocks any pending `recv_advance`.
    fn shutdown(&self);
}

#[derive(Debug)]
pub enum ClockTransportError {
    Disconnected,
    Serialization(String),
}
```

**`ZenohClockTransport`** (move Zenoh-specific logic from `ZenohClockBackend` here):
```rust
pub struct ZenohClockTransport {
    query_sender: Sender<Query>,
    // queryable kept alive for its lifetime
    _queryable: Queryable<()>,
    shutdown: Arc<AtomicBool>,
    // worker thread handle for join on drop
    worker: Option<JoinHandle<()>>,
}

impl ZenohClockTransport {
    pub fn new(session: &Arc<Session>, node_id: u32, stall_timeout_ms: u32) -> Result<Self, zenoh::Error>;
}
```

The worker thread logic (currently `zenoh_clock_worker_loop` in `zenoh-clock/src/lib.rs`)
moves into `ZenohClockTransport`. The `recv_advance` method receives from the channel;
`send_ready` packages the reply and sends it.

**`MockClockTransport`** (`#[cfg(test)]` only):
```rust
pub struct MockClockTransport {
    advance_queue: Mutex<VecDeque<ClockAdvanceReq>>,
    ready_log: Mutex<Vec<ClockReadyResp>>,
    shutdown: AtomicBool,
    cond: Condvar,
}
impl MockClockTransport {
    pub fn new() -> Self;
    pub fn push_advance(&self, req: ClockAdvanceReq);   // enqueue a request
    pub fn take_ready(&self) -> Vec<ClockReadyResp>;    // drain the reply log
}
impl ClockSyncTransport for MockClockTransport { ... }
```
`recv_advance` blocks on `cond` until an entry is in `advance_queue` or `shutdown` is set.

**Refactoring `ZenohClockBackend`**:
Change from:
```rust
pub struct ZenohClockBackend {
    pub session: Session,
    pub queryable: Option<Queryable<()>>,
    pub query_sender: Option<Sender<Query>>,
    // ...
}
```
To:
```rust
pub struct ZenohClockBackend {
    pub transport: Box<dyn ClockSyncTransport>,
    pub node_id: u32,
    pub stall_timeout_ms: u32,
    pub mutex: Mutex<()>,
    pub cond: Condvar,
    pub quantum_ready: AtomicBool,
    pub quantum_done: AtomicBool,
    pub delta_ns: AtomicU64,
    pub vtime_ns: AtomicU64,
    pub mujoco_time_ns: AtomicU64,
    pub stall_count: AtomicU64,
    pub is_first_quantum: AtomicBool,
    pub shutdown: Arc<AtomicBool>,
    // profiling fields unchanged
}
```
The vCPU-side functions `zenoh_clock_quantum_wait_internal` and
`handle_quantum_execution` call `backend.transport.recv_advance()` and
`backend.transport.send_ready()` — no Zenoh imports required at that level.

**Unit tests** (in `hw/rust/virtmcu-zenoh/src/clock_transport.rs`):
- `test_mock_transport_recv_returns_queued`: push 3 `ClockAdvanceReq`s via
  `push_advance`; call `recv_advance(Duration::from_secs(1))` 3 times; assert the
  returned values match what was pushed, in order.
- `test_mock_transport_timeout_on_empty`: call `recv_advance(Duration::from_millis(20))`
  on empty queue; assert it returns `None` and takes < 50ms wall-clock.
- `test_mock_transport_shutdown_unblocks`: start `recv_advance` on a thread, call
  `shutdown()` from main, assert thread returns within 200ms.
- `test_mock_transport_send_ready_logged`: call `send_ready(resp)`; call `take_ready()`;
  assert the resp is in the log.

**Definition of Done**:
- [ ] `ClockSyncTransport` trait exists in `hw/rust/virtmcu-zenoh/src/clock_transport.rs`.
- [ ] `ZenohClockTransport` implements it; all existing phase-7 integration tests pass.
- [ ] `MockClockTransport` exists and all 4 unit tests pass.
- [ ] `hw/rust/zenoh-clock/src/lib.rs` has zero `use zenoh::` imports in the vCPU path
  (only `ZenohClockBackend` and `ClockSyncTransport`).
- [ ] `make lint` passes.

---

### **[DET-4] `UnixSocketClockTransport`** — Single-Host Performance

**Status**: 🟢 Completed.

**Goal**: Implement `ClockSyncTransport` over a Unix domain socket, cutting the clock
RTT from 10–50 µs (Zenoh) to 1–3 µs for single-host runs. This is the default transport
when all nodes run on the same host.

**Background**: `hw/rust/remote-port/src/lib.rs` already implements the same pattern
(fixed-size binary messages over `UnixListener`). Study it before implementing.

**Wire format** (reuse `ClockAdvanceReq` / `ClockReadyResp` from `virtmcu-api`):
```
Advance request  (TA → QEMU):  16 bytes: [u64 delta_ns LE][u64 mujoco_time_ns LE]
Ready reply      (QEMU → TA):  16 bytes: [u64 current_vtime_ns LE][u32 n_frames LE][u32 error_code LE]
```
Use `ClockAdvanceReq::unpack_slice(&bytes)` and `ClockReadyResp::pack()` from
`virtmcu-api` for serialization. Never use raw `transmute` or `ptr::copy`.

**Files to create**:
- `hw/rust/virtmcu-zenoh/src/unix_clock_transport.rs`

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs` — re-export `UnixSocketClockTransport`
- `hw/rust/zenoh-clock/src/lib.rs` — add `mode=unix-socket` branch in `zenoh_clock_realize`
  and `socket` property via `define_prop_string!(c"socket".as_ptr(), ZenohClock, socket_path)`
- `tests/conftest.py` — document the new `extra_args` for unix-socket mode

**`UnixSocketClockTransport` struct** (in `unix_clock_transport.rs`):
```rust
pub struct UnixSocketClockTransport {
    socket_path: PathBuf,
    /// Channel: listener thread → recv_advance caller
    req_rx: Mutex<Receiver<(ClockAdvanceReq, UnixStream)>>,
    req_tx: Sender<(ClockAdvanceReq, UnixStream)>,
    /// Channel: send_ready caller → listener thread
    resp_tx: Sender<(ClockReadyResp, UnixStream)>,
    resp_rx: Mutex<Receiver<(ClockReadyResp, UnixStream)>>,
    shutdown: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
}
```

**Listener thread behavior**:
1. `UnixListener::bind(&socket_path)` (set `SO_REUSEADDR`).
2. Accept one connection (the TimeAuthority). Log `accepted connection`.
3. Loop:
   a. Read exactly 16 bytes → call `ClockAdvanceReq::unpack_slice(&buf[..16])`.
   b. Send `(req, stream.try_clone()?)` to `req_tx`.
   c. Wait on `resp_rx` for `(resp, stream)`.
   d. Write `resp.pack()` (16 bytes) to `stream`.
4. On read error or shutdown: break.

**Shutdown sequence**:
1. Set `AtomicBool`.
2. Connect a dummy client to `socket_path` to unblock `accept()`.
3. Join the listener thread.
4. Remove the socket file.

**New QEMU property** (in `zenoh-clock`):
```
-device zenoh-clock,mode=unix-socket,socket=/tmp/my_run/clock0.sock,node=0
```
In `zenoh_clock_realize`: if `mode_str == "unix-socket"`, construct
`UnixSocketClockTransport::new(&socket_path)` and pass it as the transport.

**Python helper** (add to `tests/helpers/unix_clock_client.py` — new file):
```python
import struct, socket

REQ_FMT = "<QQ"   # delta_ns, mujoco_time_ns
RESP_FMT = "<QII" # current_vtime_ns, n_frames, error_code

class UnixClockClient:
    def __init__(self, socket_path: str):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(socket_path)

    def advance(self, delta_ns: int, mujoco_time_ns: int = 0) -> dict:
        self.sock.sendall(struct.pack(REQ_FMT, delta_ns, mujoco_time_ns))
        data = self.sock.recv(16)
        vtime, n_frames, error = struct.unpack(RESP_FMT, data)
        return {"vtime_ns": vtime, "n_frames": n_frames, "error": error}

    def close(self): self.sock.close()
```

**Unit tests** (Rust, in `unix_clock_transport.rs`):
- `test_unix_transport_roundtrip`: create transport, spawn a thread acting as TA
  (sends a 16-byte request, reads 16-byte reply), call `recv_advance` + `send_ready`
  from test thread. Assert the delta and vtime values match. Assert wall-clock < 5ms.
- `test_unix_transport_wire_format`: assert that `send_ready` with
  `ClockReadyResp { error_code: 0, current_vtime_ns: 42, n_frames: 0 }` writes
  exactly `[42u64.to_le_bytes(), 0u32.to_le_bytes(), 0u32.to_le_bytes()].concat()`.
- `test_unix_transport_shutdown_unblocks`: start `recv_advance(Duration::from_secs(10))`
  on a thread, call `shutdown()` from main, assert the thread returns within 200ms.
- `test_unix_transport_socket_removed_on_drop`: assert the socket file is removed after
  the transport is dropped.

**Integration test** (`tests/test_det4_unix_clock.py`):
- Boot QEMU with `mode=unix-socket,socket={tmp_path}/clock.sock`.
- Use `UnixClockClient` to send 50 advance quanta of 1ms each.
- Assert firmware UART output matches expected (use the same firmware as `test_phase7`).
- Assert median round-trip < 5ms (measure 50 quanta wall-clock / 50).

**Stress test** (Rust, `tests/stress_unix_clock_transport.rs`):
- 10 000 sequential request/reply round-trips. Assert zero errors. Assert no garbled
  16-byte frame boundaries (check that each reply matches its paired request's identity).

**Definition of Done**:
- [ ] `make lint` passes.
- [ ] All 4 Rust unit tests pass.
- [ ] Stress test: 10 000 round-trips, zero errors.
- [ ] Python integration test passes in CI including ASan build.
- [ ] QEMU accepts `mode=unix-socket,socket=<path>` and boots correctly.
- [ ] `UnixClockClient` Python helper exists and is used by the integration test.
- [ ] Existing Zenoh-mode phase-7 tests still pass (no regression).

---

### **[DET-5] `DeterministicCoordinator` Quantum Barrier** — Correctness

**Status**: 🟡 Open. No dependency on DET-1..DET-4 (can be developed in parallel).

**Goal**: Promote `zenoh_coordinator` from a pass-through Zenoh switch to a per-quantum
PDES barrier that enforces canonical message ordering. This closes the global determinism
gap described in ADR-014.

**The problem**: Two messages sent at virtual time T=10ms from nodes A and B to node C
arrive at C in OS-scheduler-determined order — different on every run. The barrier
forces: wait for all nodes to finish quantum Q → sort all messages → deliver in canonical
order → start quantum Q+1.

**New binary**: `tools/deterministic_coordinator/` (new Rust binary crate)

**Files to create**:
- `tools/deterministic_coordinator/Cargo.toml`
- `tools/deterministic_coordinator/src/main.rs`
- `tools/deterministic_coordinator/src/barrier.rs`
- `tools/deterministic_coordinator/src/topology.rs` (stub; full implementation in DET-6)
- `tools/deterministic_coordinator/src/message_log.rs` (stub; full implementation in DET-8)
- `tests/test_det5_coordinator_barrier.py`

**`CoordMessage` struct** (the canonical message unit, in `barrier.rs`):
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordMessage {
    pub src_node_id: u32,
    pub dst_node_id: u32,
    pub delivery_vtime_ns: u64,
    pub sequence_number: u64,  // monotonically increasing per src_node_id, reset each quantum
    pub protocol: Protocol,
    pub payload: Vec<u8>,
}

impl Ord for CoordMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        // Canonical total order — independent of arrival order
        self.delivery_vtime_ns.cmp(&other.delivery_vtime_ns)
            .then_with(|| self.src_node_id.cmp(&other.src_node_id))
            .then_with(|| self.sequence_number.cmp(&other.sequence_number))
    }
}
impl PartialOrd for CoordMessage { fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) } }
```

**`QuantumBarrier` struct** (in `barrier.rs`):
```rust
pub struct QuantumBarrier {
    n_nodes: usize,
    done_count: AtomicUsize,           // incremented by each node on quantum completion
    message_buffer: Mutex<Vec<CoordMessage>>,
    all_done_cond: Condvar,            // signaled when done_count == n_nodes
}

impl QuantumBarrier {
    pub fn new(n_nodes: usize) -> Self;

    /// Called by each node when its quantum is complete.
    /// When the N-th node calls this, the barrier sorts and returns all messages.
    pub fn submit_done(&self, node_id: u32, messages: Vec<CoordMessage>)
        -> Option<Vec<CoordMessage>>;  // Some(sorted) when all N nodes done, None otherwise

    /// Reset for the next quantum. Called after delivery is complete.
    pub fn reset(&self);

    /// Block until all N nodes have submitted. Returns sorted messages.
    pub fn wait_for_all(&self, timeout: Duration) -> Result<Vec<CoordMessage>, BarrierError>;
}
```

**Coordinator Zenoh topics** (new, separate from existing `sim/eth/frame/` topics):
```
sim/coord/{node_id}/tx      — node publishes outbound messages (batch, per quantum)
sim/coord/{node_id}/done    — node signals quantum complete (payload: quantum_number u64)
sim/coord/{node_id}/rx      — coordinator publishes inbound messages (sorted)
sim/coord/{node_id}/start   — coordinator signals quantum Q+1 can begin
```

**`main.rs` structure**:
1. Parse CLI args: `--nodes <N>`, `--router <addr>`, `--topology <yaml>` (stub for DET-6).
2. Open a Zenoh session (client mode, explicit router).
3. Subscribe to `sim/coord/*/tx` and `sim/coord/*/done`.
4. For each `done` message: call `barrier.submit_done(node_id, messages)`.
5. When `submit_done` returns `Some(sorted_messages)`: deliver each message to its
   `dst_node_id` via publish to `sim/coord/{dst}/rx`; publish `start` to all nodes.
6. Call `barrier.reset()`.

**Unit tests** (in `tools/deterministic_coordinator/src/barrier.rs`):
- `test_barrier_waits_for_all_3_nodes`: create barrier with N=3; call `submit_done` for
  nodes 0 and 1 with empty message lists; assert `submit_done` returns `None` both times;
  call `submit_done` for node 2; assert returns `Some(_)`.
- `test_canonical_sort_same_vtime`: insert messages with vtime=10ms from nodes [2,0,1]
  each with seq=0; assert sorted order is [src=0, src=1, src=2].
- `test_canonical_sort_different_vtime`: insert 3 messages at vtime [30ms, 10ms, 20ms];
  assert sorted order is [10ms, 20ms, 30ms].
- `test_barrier_reset_allows_next_quantum`: complete a quantum (N done signals), call
  `reset()`, assert `done_count == 0` and a new round can begin.
- `test_barrier_duplicate_done_rejected`: call `submit_done` for node 0 twice; assert the
  second call returns an error or panics (duplicate done is a protocol violation).

**Stress test** (Rust, `tools/deterministic_coordinator/tests/stress_barrier.rs`):
- With N=10 nodes, run 1000 quantum rounds. In each round, spawn 10 threads each calling
  `submit_done` concurrently with 10 random messages. Assert exactly one `Some` is
  returned per round. Assert the returned sorted list is correctly ordered every round.
  Use `cargo test --release -- stress_barrier`. Zero failures required.

**Integration test** (`tests/test_det5_coordinator_barrier.py`):
- Launch 3 QEMU nodes + coordinator (with `--nodes 3`).
- All 3 nodes send a message to each other in the same quantum at vtime=5ms.
- Capture delivery order at each recipient by checking coordinator stdout or a log.
- Assert all messages are delivered (9 total: 3 nodes × 3 recipients including self is
  topology-dependent; adjust based on DET-6 topology; for now allow all-to-all).
- Run the scenario **100 times**. Assert every run produces the **identical** delivery
  sequence (compare delivery-order log line by line).

**Definition of Done**:
- [ ] `cargo test -p deterministic_coordinator` passes all 5 unit tests.
- [ ] Stress test: 1000 rounds × 10 nodes, zero errors.
- [ ] Python integration test: 100/100 runs produce identical delivery sequence.
- [ ] Existing phase-7 and phase-13 CI tests still pass (coordinator is optional for
  single-node tests; existing tests do not use it).
- [ ] `make lint` passes.

---

### **[DET-6] Topology-First YAML Loading** — Correctness

**Status**: 🟡 Open. Depends on: DET-5 complete.

**Goal**: The full network graph is declared in the world YAML `topology:` section. The
coordinator enforces the graph: messages not permitted by the graph are dropped. Topology
changes (mobile nodes) are pushed by the physics engine before each quantum.

**Files to modify**:
- `tools/yaml2qemu.py` — add `topology:` section schema validation
- `tools/deterministic_coordinator/src/topology.rs` — implement from stub
- `worlds/*.yaml` — add `topology:` section to all existing worlds

**YAML schema to add** (under existing world YAML `nodes:` section):
```yaml
topology:
  global_seed: 0              # u64; default 0; seeds all stochastic protocols
  links:
    - type: ethernet           # values: ethernet, uart, spi, canfd, flexray
      nodes: [0, 1]            # bidirectional; list exactly 2 node IDs
      baud: null               # optional; required for uart/spi
    - type: uart
      nodes: [0, 2]
      baud: 115200
  wireless:                    # optional; omit if no RF nodes
    medium: 802154             # values: 802154, ble, wifi
    nodes:
      - id: 0
        initial_position: [0.0, 0.0, 0.0]   # x, y, z in meters
      - id: 1
        initial_position: [1.0, 0.0, 0.0]
    max_range_m: 10.0
```

**`TopologyGraph` API** (in `topology.rs`):
```rust
pub struct TopologyGraph {
    pub global_seed: u64,
    wire_links: Vec<WireLink>,
    wireless_medium: Option<WirelessMedium>,
    node_positions: HashMap<u32, [f64; 3]>,
    max_wireless_range_m: f64,
}

impl TopologyGraph {
    /// Load from a world YAML file.
    pub fn from_yaml(path: &Path) -> Result<Self, TopologyError>;

    /// Returns true if a message from `src` to `dst` over `protocol` is permitted.
    pub fn is_link_allowed(&self, src: u32, dst: u32, protocol: Protocol) -> bool;

    /// Called by physics engine before each quantum with updated node positions.
    pub fn update_positions(&mut self, updates: &[(u32, [f64; 3])]);

    /// Current RF-visible neighbors of `node_id` given `update_positions`.
    pub fn rf_neighbors(&self, node_id: u32) -> Vec<u32>;
}
```

**Coordinator integration**: In `barrier.rs`, after sorting messages, call
`topology.is_link_allowed(msg.src_node_id, msg.dst_node_id, msg.protocol)` for each
message. Drop messages that return false and write them to the PCAP log as topology
violations (protocol field: `PROTOCOL_TOPO_VIOLATION`).

**Validation rules enforced by `yaml2qemu.py`**:
1. Every node ID in `topology.links[].nodes` must match a node ID in `nodes:`.
2. Every node ID in `topology.wireless.nodes[].id` must match a node ID in `nodes:`.
3. `global_seed` must be a non-negative integer.
4. Fail with a human-readable error message if any rule is violated (do not silently ignore).

**Unit tests** (Rust):
- `test_wire_link_bidirectional`: YAML with `links: [{type: ethernet, nodes: [0, 1]}]`;
  assert `is_link_allowed(0, 1, Ethernet)` and `is_link_allowed(1, 0, Ethernet)` both true.
- `test_wire_link_no_cross_protocol`: same YAML; assert `is_link_allowed(0, 1, UART)` false.
- `test_wireless_in_range`: place node 0 at [0,0,0], node 1 at [5,0,0], max_range=10;
  assert `rf_neighbors(0)` contains 1.
- `test_wireless_out_of_range`: place node 1 at [15,0,0]; assert `rf_neighbors(0)`
  does NOT contain 1.
- `test_position_update_changes_neighbors`: start node 1 at [15,0,0] (out of range);
  call `update_positions([(1, [5.0, 0.0, 0.0])])`; assert `rf_neighbors(0)` now contains 1.
- `test_topology_unknown_node_rejected`: YAML references node ID 99 not in `nodes:`;
  assert `from_yaml` returns `Err(TopologyError::UnknownNode(99))`.

**Integration test** (`tests/test_det6_topology.py`):
- World YAML with 3 nodes: only a UART link between 0 and 1 (no link to node 2).
- Boot 3 QEMU nodes + coordinator.
- Send an Ethernet frame from node 0 to node 2 (not in the graph).
- Assert the frame is NOT delivered to node 2's firmware (UART output unchanged).
- Assert the coordinator log contains a topology violation entry for this message.
- Send a UART message from node 0 to node 1; assert it IS delivered.

**Definition of Done**:
- [ ] `yaml2qemu.py` validates `topology:` and fails on unknown node IDs.
- [ ] All 6 Rust unit tests pass.
- [ ] Python integration test passes (topology violation enforced).
- [ ] All `worlds/*.yaml` files have a `topology:` section (add empty `links: []` to any
  that do not model multi-node communication).
- [ ] `make lint` passes.

---

### **[DET-7] Deterministic CSMA/CA Seeding** — Correctness for Wireless

**Status**: 🟡 Open. Depends on: DET-6 (for `global_seed` in YAML). Also requires
Phase 29.4 (CSMA/CA backoff timers) to be complete.

**Goal**: All stochastic protocol behavior is seeded deterministically. Two runs with the
same world YAML produce identical backoff sequences, collision outcomes, and channel
access patterns.

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs` — add `seed_for_quantum` utility function
- `hw/rust/zenoh-802154/src/lib.rs` — replace any `thread_rng()` with seeded PRNG
- `hw/rust/zenoh-canfd/src/lib.rs` — audit and fix if stochastic behavior exists

**`seed_for_quantum` utility** (add to `virtmcu-zenoh/src/lib.rs`):
```rust
/// Compute the deterministic per-node, per-quantum PRNG seed.
/// This is the ONLY approved seed source for all stochastic simulation in virtmcu.
/// Do NOT call rand::thread_rng() or use SystemTime in simulation code.
pub fn seed_for_quantum(global_seed: u64, node_id: u32, quantum_number: u64) -> u64 {
    const SALT_NODE: u64 = 0x9e37_79b9_7f4a_7c15;    // golden ratio
    const SALT_QUANTUM: u64 = 0x6c62_272e_07bb_0142;  // FNV prime
    global_seed
        ^ u64::from(node_id).wrapping_mul(SALT_NODE)
        ^ quantum_number.wrapping_mul(SALT_QUANTUM)
}
```

**Changes to `zenoh-802154`**:
1. Add `global_seed: u64` field, populated from QOM property `seed` (u64, default 0).
2. Add `quantum_number: Arc<AtomicU64>` field, incremented on each quantum advance signal.
3. In the CSMA/CA backoff computation, replace any call to `rand::thread_rng()` with:
   ```rust
   let seed = virtmcu_zenoh::seed_for_quantum(
       self.global_seed, self.node_id, self.quantum_number.load(Ordering::Relaxed));
   let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
   let backoff_slots: u32 = rng.gen_range(0..=max_backoff_slots);
   ```
4. The `quantum_number` is incremented by subscribing to `sim/clock/heartbeat/{node_id}`
   or by integrating with the `ClockSyncTransport` (preferred in DET-3 world).

**Banned patterns** (enforce with grep in `make lint`):
- `grep -r "thread_rng\|SystemTime::now" hw/rust/ --include="*.rs"` must find zero matches.
  Any exception requires `// PRNG_EXCEPTION: <reason>` comment and grep allowlist entry.

**Unit tests** (Rust):
- `test_seed_for_quantum_pure`: call `seed_for_quantum(42, 1, 100)` twice; assert results
  are identical (pure function, no side effects).
- `test_seed_for_quantum_node_isolation`: assert `seed_for_quantum(42, 0, 0)` ≠
  `seed_for_quantum(42, 1, 0)` — different nodes produce different seeds.
- `test_seed_for_quantum_quantum_isolation`: assert `seed_for_quantum(42, 0, 0)` ≠
  `seed_for_quantum(42, 0, 1)` — different quanta produce different seeds.
- `test_seed_for_quantum_global_isolation`: assert `seed_for_quantum(0, 0, 0)` ≠
  `seed_for_quantum(1, 0, 0)` — different global seeds produce different results.
- `test_csma_backoff_reproducible`: create a `SmallRng` seeded with `seed_for_quantum(42,
  0, 5)` twice; generate 50 random values from each; assert they are identical.

**Integration test** (`tests/test_det7_csma_determinism.py`):
- World YAML: 3 nodes with 802.15.4 medium, `global_seed: 99999`.
- Boot 3 QEMU nodes + coordinator. All 3 transmit simultaneously (guaranteed collision
  scenario) for 30 quanta.
- Capture each node's successful/failed transmission sequence via UART or coordinator log.
- Run the **identical** scenario again with `global_seed: 99999`.
- Assert the transmission outcome sequence is **byte-identical** across both runs.
- Run once more with `global_seed: 11111`; assert the outcome sequence is **different**
  from the `99999` runs (confirms seed has real effect on behavior).

**Definition of Done**:
- [ ] `seed_for_quantum` exists in `virtmcu-zenoh` and is documented.
- [ ] All 5 unit tests pass.
- [ ] `grep -r "thread_rng\|SystemTime::now" hw/rust/ --include="*.rs"` finds zero
  matches outside of test-only code.
- [ ] Integration test: identical outcomes for same seed, different for different seed.
- [ ] `make lint` passes.

---

### **[DET-8] Unified PCAP Logging Side Channel** — Observability

**Status**: 🟡 Open. Depends on: DET-5 complete.

**Goal**: The coordinator writes a libpcap-format log of every inter-node message with
its virtual timestamp. Two runs with the same seed produce byte-identical PCAP files.
This file is the CI determinism oracle and the feed for the Wireshark plugin (DET-9).

**Files to modify**:
- `tools/deterministic_coordinator/src/message_log.rs` — implement from stub
- `tools/deterministic_coordinator/src/main.rs` — integrate log writer, add `--pcap-log`
  CLI argument
- `tools/deterministic_coordinator/src/barrier.rs` — call `log.write_message()` after
  sort, before delivery

**libpcap global header** (write once at file open, 24 bytes total, all LE):
```
magic_number:  u32 = 0xa1b2c3d4
version_major: u16 = 2
version_minor: u16 = 4
thiszone:      i32 = 0
sigfigs:       u32 = 0
snaplen:       u32 = 65535
network:       u32 = 147    // DLT_USER0; virtmcu custom link type
```

**libpcap packet header** (16 bytes per packet, all LE):
```
ts_sec:   u32 = delivery_vtime_ns / 1_000_000_000
ts_usec:  u32 = (delivery_vtime_ns % 1_000_000_000) / 1000
incl_len: u32 = payload.len() + 10  // 4 (src) + 4 (dst) + 2 (protocol) + payload
orig_len: u32 = incl_len
```

**Packet payload** (immediately follows header):
```
src_node_id: u32 LE
dst_node_id: u32 LE
protocol_id: u16 LE  (1=Ethernet, 2=UART, 3=802154, 4=CAN-FD, 5=FlexRay, 255=TopoViolation)
...original frame bytes...
```

**`MessageLog` API** (in `message_log.rs`):
```rust
pub struct MessageLog {
    writer: BufWriter<File>,
}

impl MessageLog {
    /// Create a new PCAP file at `path`, writing the global header immediately.
    pub fn create(path: &Path) -> Result<Self, io::Error>;

    /// Append one message to the log. `msg.payload` is the raw frame bytes.
    pub fn write_message(&mut self, msg: &CoordMessage) -> Result<(), io::Error>;

    /// Flush the internal buffer to disk.
    pub fn flush(&mut self) -> Result<(), io::Error>;
}
```

**CLI argument**: Add `--pcap-log <path>` to `main.rs`. If not provided, no log is written.
In CI, the Python integration tests pass `--pcap-log {tmp_path}/sim.pcap`.

**Unit tests** (Rust, in `message_log.rs`):
- `test_pcap_global_header_bytes`: create a `MessageLog` writing to a `Vec<u8>`;
  assert the first 24 bytes exactly match the magic number, version, and DLT_USER0
  constants listed above (compare byte arrays).
- `test_pcap_packet_timestamp_1500ms`: write a message with `delivery_vtime_ns =
  1_500_000_000`; assert `ts_sec == 1` and `ts_usec == 500_000` in the packet header.
- `test_pcap_payload_node_ids`: write a message with `src=2, dst=5, protocol=UART`;
  assert bytes 16..20 of file = `[2,0,0,0]` and bytes 20..24 = `[5,0,0,0]` and
  bytes 24..26 = `[2,0]` (UART protocol_id = 2).
- `test_pcap_messages_in_sort_order`: write 3 messages with vtimes [30ms, 10ms, 20ms]
  in insertion order; assert the PCAP file contains them in vtime-ascending order
  (the barrier sorts before calling `write_message`, so the log writer receives them
  already sorted — verify this contract).

**Integration test** (`tests/test_det8_pcap_determinism.py`):
- Run a 2-node, 20-quantum simulation with `--pcap-log {tmp_path}/run1.pcap`.
- Run the identical simulation again with `--pcap-log {tmp_path}/run2.pcap`.
- Assert `run1.pcap == run2.pcap` (binary comparison: `open(f1,'rb').read() == open(f2,'rb').read()`).
- Open the PCAP with `dpkt` (add to `requirements.txt` if not present); assert:
  - Packet count matches expected inter-node message count.
  - All `ts_sec`/`ts_usec` timestamps are non-decreasing.
  - Magic number is correct.

**Definition of Done**:
- [ ] PCAP file opens without error in Wireshark (manual verification with sample run).
- [ ] All 4 Rust unit tests pass.
- [ ] Python integration test: two runs produce byte-identical PCAP files.
- [ ] `make lint` passes.

---

### **[DET-9] Wireshark Extcap Plugin** — Observability / Tooling

**Status**: 🟡 Open. Depends on: DET-8 complete. Lowest priority; implement last.

**Goal**: A Wireshark extcap plugin reads the coordinator PCAP log and displays each
inter-node message with virtual timestamp and custom protocol dissection.

**Note**: This is UI tooling. It does not affect simulation correctness or performance.
Do not block other DET phases on this.

**Files to create**:
- `tools/wireshark-extcap/virtmcu_extcap.py` — Python extcap interface script
- `tools/wireshark-extcap/virtmcu.lua` — Lua protocol dissector
- `docs/WIRESHARK_SETUP.md` — installation instructions

**`virtmcu_extcap.py`** implements the [Wireshark extcap protocol](https://www.wireshark.org/docs/wsdg_html_chunked/ChCaptureExtcap.html):

When called with `--extcap-interfaces`, print:
```
extcap {version=1.0}{help=https://github.com/virtmcu}
interface {value=virtmcu-coordinator}{display=VirtMCU Coordinator}
```

When called with `--extcap-dlts --extcap-interface virtmcu-coordinator`, print:
```
dlt {number=147}{name=VIRTMCU}{display=VirtMCU Protocol}
```

When called with `--capture --extcap-interface virtmcu-coordinator --fifo <path>
--pcap-log <logfile>`: open `logfile`, read it as a PCAP file, write each packet to
`fifo` in PCAP format. For a live capture, use `tail -c +0 -f` semantics to stream
as new packets are appended.

**`virtmcu.lua`** dissector (registered for DLT 147):
```lua
local virtmcu_proto = Proto("virtmcu", "VirtMCU Inter-Node Message")
local f_src  = ProtoField.uint32("virtmcu.src_node",  "Source Node",      base.DEC)
local f_dst  = ProtoField.uint32("virtmcu.dst_node",  "Destination Node", base.DEC)
local f_proto = ProtoField.uint16("virtmcu.protocol", "Protocol",          base.DEC,
    {[1]="Ethernet",[2]="UART",[3]="802.15.4",[4]="CAN-FD",[5]="FlexRay",[255]="TopoViolation"})
virtmcu_proto.fields = { f_src, f_dst, f_proto }

function virtmcu_proto.dissector(buffer, pinfo, tree)
    pinfo.cols.protocol = "VIRTMCU"
    local subtree = tree:add(virtmcu_proto, buffer(), "VirtMCU Message")
    subtree:add_le(f_src,   buffer(0, 4))
    subtree:add_le(f_dst,   buffer(4, 4))
    subtree:add_le(f_proto, buffer(8, 2))
    -- Hand remaining bytes to Ethernet/UART dissectors based on protocol_id
    local proto_id = buffer(8, 2):le_uint()
    if proto_id == 1 then
        Dissector.get("eth_withoutfcs"):call(buffer(10):tvb(), pinfo, tree)
    end
end
DissectorTable.get("wtap_encap"):add(147, virtmcu_proto)
```

**Unit tests** (Python):
- `test_extcap_interfaces_output`: run `python3 virtmcu_extcap.py --extcap-interfaces`;
  assert stdout contains `interface {value=virtmcu-coordinator}`.
- `test_extcap_dlts_output`: run with `--extcap-dlts --extcap-interface virtmcu-coordinator`;
  assert stdout contains `dlt {number=147}`.
- `test_extcap_reads_pcap_file`: create a minimal valid 3-packet PCAP with known content;
  run extcap in capture mode writing to a temp FIFO; assert the output bytes match the
  input PCAP bytes exactly (extcap is a pass-through for replay mode).

**Definition of Done**:
- [ ] `virtmcu_extcap.py --extcap-interfaces` and `--extcap-dlts` pass Wireshark's
  extcap interface validation (run `wireshark -D` and verify `virtmcu-coordinator` appears).
- [ ] Lua dissector shows `src_node`, `dst_node`, `protocol` fields in Wireshark's
  packet detail pane for a sample PCAP (manual verification).
- [ ] All 3 Python unit tests pass.
- [ ] `WIRESHARK_SETUP.md` has complete step-by-step installation instructions.
- [ ] `make lint` (ruff) passes on `tools/wireshark-extcap/`.

---

## 4. Architectural Hardening — Concurrency, Correctness & Scale

> **Purpose**: Close known concurrency bugs, wire-protocol gaps, and design debt identified
> in the April 2026 deep-architecture review. Tasks are ordered by severity. Each is
> self-contained with exact file paths, step-by-step implementation, tests, and a binary
> definition of done.
>
> **Audience**: AI coding agents and junior engineers. Follow steps exactly. Do not infer.
>
> **Prerequisite**: `make lint && make test-unit` MUST pass before starting any task.

---

### **[ARCH-1] Fix `GLOBAL_CLOCK` TOCTOU Race** — P0 Safety / Correctness

**Status**: 🟢 Completed.

**Goal**: `ACTIVE_HOOKS` is incremented *after* `GLOBAL_CLOCK` is loaded, creating a
window where the pointer is read, finalize runs and frees the allocation, and the hook
then dereferences freed memory. Fix by: (a) incrementing `ACTIVE_HOOKS` *before* loading
the pointer and checking it for null, or (b) replacing the raw `AtomicPtr<ZenohClock>` with
a `Mutex<Option<Weak<ZenohClock>>>` that keeps the allocation alive.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — `zenoh_clock_quantum_hook` function

**Step-by-step implementation** (Option A — increment-first, lower diff):

1. In `zenoh_clock_quantum_hook` (or equivalent hook entry point), reorder operations:
   ```rust
   // Step 1: Announce entry BEFORE loading the pointer.
   ACTIVE_HOOKS.fetch_add(1, Ordering::AcqRel);

   // Step 2: Load the pointer AFTER announcing entry.
   let clock_ptr = GLOBAL_CLOCK.load(Ordering::Acquire);

   // Step 3: If null, device was already finalized — exit cleanly.
   if clock_ptr.is_null() {
       ACTIVE_HOOKS.fetch_sub(1, Ordering::AcqRel);
       return;
   }

   // SAFETY: pointer is non-null and ACTIVE_HOOKS > 0 prevents finalize from
   // freeing the allocation while we hold a reference.
   let clock = unsafe { &*clock_ptr };
   // ... use clock ...

   // Step 4: Announce exit.
   ACTIVE_HOOKS.fetch_sub(1, Ordering::AcqRel);
   ```

2. In `zenoh_clock_finalize` (device teardown), the existing sequence of:
   - Set `GLOBAL_CLOCK` to null (`store(null, Ordering::Release)`)
   - Spin-wait for `ACTIVE_HOOKS == 0`

   remains correct *because* any hook that read a non-null pointer already incremented
   `ACTIVE_HOOKS` before we stored null. After null is stored, no new hooks will pass
   step 3. Add a `compiler_fence` between the null store and the spin-wait:
   ```rust
   GLOBAL_CLOCK.store(core::ptr::null_mut(), Ordering::Release);
   core::sync::atomic::compiler_fence(Ordering::SeqCst);
   // Now wait for any in-flight hooks to exit.
   while ACTIVE_HOOKS.load(Ordering::Acquire) > 0 {
       std::thread::yield_now();
   }
   ```

**Unit tests** (Rust, in `hw/rust/zenoh-clock/src/lib.rs`):
- `test_hook_increment_before_load`: Use `loom` (if state space is feasible) or a custom
  `AtomicPtr` spy to assert that `ACTIVE_HOOKS.fetch_add` happens-before the pointer load
  in the hook. Alternatively: a stress test that races `zenoh_clock_finalize` against 100
  concurrent hook invocations; assert no ASAN/TSan report.

**Stress test**:
- `stress_hook_vs_finalize_race`: spawn 50 threads each calling the hook 200 times;
  simultaneously call finalize once after 5 ms. Run under TSan (`RUSTFLAGS="-Z sanitizer=thread"`).
  Assert zero data-race reports and no null-pointer dereferences. 1000 iterations total.

**Definition of Done**:
- [x] `ACTIVE_HOOKS.fetch_add` is unconditionally the first line of the hook body.
- [x] TSan stress test: 1000 iterations, zero reports.
- [x] `cargo miri test -p zenoh-clock -- hook` passes.
- [x] `make lint` passes.

---

### **[ARCH-2] Replace Manual BQL unlock with RAII in Clock Halt Callback** — Safety

**Status**: 🟢 Completed.

**Goal**: `zenoh_clock_cpu_halt_cb_internal` (or equivalent) calls `virtmcu_bql_unlock()`
and `virtmcu_bql_lock()` directly. If anything panics or early-returns between those two
calls, the BQL is permanently lost — deadlocking QEMU. Replace with
`Bql::temporary_unlock()` which returns an RAII guard.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — any function that manually calls `bql_unlock`/`bql_lock`

**Implementation**:
1. Find every call site matching `virtmcu_bql_unlock()` followed later by `virtmcu_bql_lock()`.
2. Replace the pair with:
   ```rust
   let _bql_unlock = Bql::temporary_unlock();
   // ... blocking Zenoh/socket call ...
   // BQL is automatically re-acquired when _bql_unlock drops at end of scope.
   ```
3. Ensure no `return` or `?` operator can escape between the old `unlock` and `lock` calls
   (the RAII guard makes this a non-issue).

**Unit tests**:
- `test_bql_reacquired_after_transport_error`: use `MockClockTransport` (DET-3) that
  returns an error from `recv_advance`; assert that after the error path returns, a
  subsequent `Bql::lock()` call succeeds (i.e., the BQL was not left dropped).

**Definition of Done**:
- [x] Zero occurrences of `virtmcu_bql_unlock()` / `virtmcu_bql_lock()` in `zenoh-clock/src/lib.rs`
  outside of `virtmcu-qom/src/sync.rs`.
- [x] `grep -n "virtmcu_bql_unlock\|virtmcu_bql_lock" hw/rust/zenoh-clock/src/lib.rs`
  finds zero matches.
- [x] `make lint` passes.

---

### **[ARCH-3] Replace Two-AtomicBool State Machine with Atomic Enum** — Correctness

**Status**: 🟢 Completed.

**Goal**: The current `quantum_ready: AtomicBool` + `quantum_done: AtomicBool` pair
allows the illegal state `(true, true)` — both set simultaneously. A single atomic enum
with compare-and-exchange transitions eliminates illegal states entirely.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — `ZenohClockBackend` struct and all transition sites

**Enum definition** (add to `hw/rust/zenoh-clock/src/lib.rs`):
```rust
/// State machine for a single clock quantum.
/// Transitions: Idle → WaitingForTA → Advancing → Done → Idle
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuantumState {
    Idle         = 0,  // no quantum in progress
    WaitingForTA = 1,  // vCPU halted; waiting for advance request from TA
    Advancing    = 2,  // TA replied; TCG executing until TB boundary
    Done         = 3,  // TB boundary reached; ready reply sent
}
```

Use `AtomicU8` with `compare_exchange` for all transitions:
```rust
static QUANTUM_STATE: AtomicU8 = AtomicU8::new(QuantumState::Idle as u8);

fn transition(from: QuantumState, to: QuantumState) -> Result<(), ()> {
    QUANTUM_STATE
        .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| ())
}
```

All former `quantum_ready.store(true, ...)` / `quantum_done.store(true, ...)` calls become
`transition(From, To)?` with an explicit panic or log on illegal transition.

**Unit tests** (use `loom` for state-space coverage):
- `test_quantum_state_valid_transitions`: assert all 4 legal transitions succeed.
- `test_quantum_state_illegal_idle_to_done`: assert `transition(Idle, Done)` returns `Err`.
- `test_quantum_state_concurrent_advance`: loom test — 2 threads try to transition
  `WaitingForTA → Advancing` simultaneously; assert exactly one succeeds.

**Stress test**:
- `stress_quantum_state_machine`: 10 000 quantum cycles end-to-end using `MockClockTransport`.
  Assert zero `Err` returns from valid transitions.

**Definition of Done**:
- [x] `quantum_ready: AtomicBool` and `quantum_done: AtomicBool` fields removed from
  `ZenohClockBackend`.
- [x] `QUANTUM_STATE: AtomicU8` replaces both.
- [x] All 3 unit tests pass; loom test runs to completion.
- [x] Stress: 10 000 cycles, zero errors.
- [x] `make lint` passes.

---

### **[ARCH-4] Add `sequence_number` to `ZenohFrameHeader`** — Wire Protocol

**Status**: 🟢 Completed.

**Goal**: The `ZenohFrameHeader` in `virtmcu-api` is missing a `sequence_number: u64`
field. Without it, the `DeterministicCoordinator` cannot implement canonical
`(vtime_ns, src_node_id, seq_num)` ordering — same-timestamp messages from the same node
would have arbitrary order.

**Files to modify**:
- `hw/rust/virtmcu-api/src/lib.rs` — `ZenohFrameHeader` struct, `pack()`, `unpack_slice()`
- `hw/rust/zenoh-chardev/src/lib.rs` — TX path: increment and set `sequence_number`
- `hw/rust/zenoh-netdev/src/lib.rs` — TX path: increment and set `sequence_number`
- `hw/rust/zenoh-802154/src/lib.rs` — TX path (if ZenohFrameHeader is used)
- `hw/rust/zenoh-canfd/src/lib.rs` — TX path (if ZenohFrameHeader is used)

**Updated `ZenohFrameHeader`**:
```rust
/// Wire header for all inter-node virtmcu messages.
/// Layout (all LE): [delivery_vtime_ns: u64][sequence_number: u64][size: u32]
/// Total: 20 bytes.
#[repr(C)]
pub struct ZenohFrameHeader {
    pub delivery_vtime_ns: u64,
    pub sequence_number: u64,   // NEW: monotonically increasing per (src_node, quantum)
    pub size: u32,
}

impl ZenohFrameHeader {
    pub const SIZE: usize = 20;  // updated from 12

    pub fn pack(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..8].copy_from_slice(&self.delivery_vtime_ns.to_le_bytes());
        buf[8..16].copy_from_slice(&self.sequence_number.to_le_bytes());
        buf[16..20].copy_from_slice(&self.size.to_le_bytes());
        buf
    }

    pub fn unpack_slice(bytes: &[u8]) -> Result<Self, HeaderError> {
        if bytes.len() < Self::SIZE {
            return Err(HeaderError::TooShort { got: bytes.len(), expected: Self::SIZE });
        }
        Ok(Self {
            delivery_vtime_ns: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            sequence_number:   u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            size:              u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
        })
    }
}
```

Each sender maintains a `per_quantum_seq: AtomicU64` field (reset to 0 at each quantum
start signal) and does `sequence_number: per_quantum_seq.fetch_add(1, Ordering::Relaxed)`.

**Unit tests** (in `virtmcu-api/src/lib.rs`):
- `test_header_roundtrip`: pack a header with `delivery_vtime_ns=12345, sequence_number=7, size=100`;
  unpack; assert all fields match.
- `test_header_size_20`: assert `ZenohFrameHeader::SIZE == 20`.
- `test_header_le_bytes`: assert byte 0 of the packed form of `delivery_vtime_ns=1` is `0x01`.
- `test_header_seq_ordering`: create 5 frames with seq [4,2,0,3,1]; sort by `ZenohFrameHeader`'s
  natural comparison (by `sequence_number`); assert order [0,1,2,3,4].

**Definition of Done**:
- [x] `ZenohFrameHeader::SIZE == 20` and `pack/unpack` round-trips correctly.
- [x] All TX paths set `sequence_number` from a per-node per-quantum atomic counter.
- [x] All 4 unit tests pass.
- [x] `make lint` passes.
- [x] Existing integration tests updated to handle the new 20-byte header.

---

### **[ARCH-5] Admission Control for `DeterministicCoordinator`** — Reliability

**Status**: 🟢 Completed.

**Goal**: A misbehaving or flooded node can send unlimited messages in one quantum,
consuming unbounded coordinator memory. Add per-node, per-quantum message limits defined
in the world YAML topology. Messages exceeding the limit are dropped and logged as
`PROTOCOL_TOPO_VIOLATION` in the PCAP log.

**Files to modify**:
- `tools/deterministic_coordinator/src/barrier.rs` — add `max_messages_per_node: usize`
  to `QuantumBarrier`; enforce in `submit_done`
- `tools/deterministic_coordinator/src/topology.rs` — add `max_messages_per_node_per_quantum`
  field to `TopologyGraph` (default: 1024)
- World YAML schema (in `tools/yaml2qemu.py`) — add optional field
  `topology.max_messages_per_node_per_quantum: u32`

**Implementation in `submit_done`**:
```rust
pub fn submit_done(
    &self,
    node_id: u32,
    mut messages: Vec<CoordMessage>,
) -> Option<Vec<CoordMessage>> {
    if messages.len() > self.max_messages_per_node {
        let excess = messages.len() - self.max_messages_per_node;
        tracing::warn!(
            "Node {} exceeded per-quantum message limit ({} > {}); dropping {} messages",
            node_id, messages.len(), self.max_messages_per_node, excess
        );
        messages.truncate(self.max_messages_per_node);
        // Dropped messages are emitted as TOPO_VIOLATION in PCAP by the caller.
    }
    // ... existing barrier logic ...
}
```

**Unit tests**:
- `test_admission_control_drops_excess`: create barrier with `max_messages_per_node=3`;
  submit 5 messages for node 0; assert only 3 appear in the returned sorted list.
- `test_admission_control_within_limit`: submit 3 messages (at limit); assert all 3 appear.
- `test_admission_control_zero_messages`: submit 0 messages; assert barrier handles correctly.

**Definition of Done**:
- [x] `max_messages_per_node_per_quantum` is a YAML field with default 1024.
- [x] Excess messages are dropped (not panicked) and a warning is logged.
- [x] All 3 unit tests pass.
- [x] `make lint` passes.

---

### **[ARCH-6] Virtual-Time Overshoot Compensation in `TimeAuthority`** — Correctness

**Status**: 🟢 Completed.

**Goal**: In `slaved-suspend` mode, the TCG hook fires at a translation-block boundary,
not at the exact nanosecond of the quantum end. QEMU may overshoot the requested quantum
by up to one TB duration. If uncompensated, cumulative overshoot drifts the simulation
clock away from the intended timeline. The `TimeAuthority` must track this drift and
subtract it from the next quantum.

**Background**: If TA requests delta=10ms and QEMU reports `current_vtime_ns` corresponding
to 10.002ms advanced, the overshoot is 2µs. The next advance should request `10ms - 2µs`.

**Files to modify**:
- `tests/time_authority.py` (or wherever `VirtualTimeAuthority`/`TimeAuthority` lives) —
  add overshoot compensation logic

**Implementation** (pseudocode — translate to actual TA class):
```python
class VirtualTimeAuthority:
    def __init__(self, ...):
        self._expected_vtime_ns: int = 0
        self._overshoot_ns: int = 0

    def step(self, delta_ns: int, ...) -> ClockReadyResp:
        # Compensate for accumulated overshoot from previous quantum.
        adjusted_delta = max(0, delta_ns - self._overshoot_ns)
        resp = self._send_advance(adjusted_delta)

        # Compute new overshoot: how much did QEMU overshoot?
        self._expected_vtime_ns += delta_ns
        self._overshoot_ns = max(0, resp.current_vtime_ns - self._expected_vtime_ns)
        return resp
```

**Unit tests** (Python, add to `tests/test_time_authority_unit.py`):
- `test_no_overshoot_when_exact`: mock transport returns `current_vtime_ns` equal to
  exactly the requested advance; assert `_overshoot_ns == 0` after the step.
- `test_overshoot_subtracted_next_step`: first step requests 10ms, mock returns 10.002ms;
  second step requests 10ms; assert actual `adjusted_delta` sent to transport is
  `9 998 000 ns` (10ms − 2µs).
- `test_overshoot_never_negative`: mock returns `current_vtime_ns` *less* than expected
  (undershoot, which should not happen but must not crash); assert `_overshoot_ns == 0`
  (clamped to zero, never negative).
- `test_1000_quantum_drift_under_1_quantum`: run 1000 step calls each requesting 1ms,
  with mock returning +100ns overshoot each time. Assert final `_expected_vtime_ns -
  actual_sum_of_adjusted_deltas < 1_000_000 ns` (drift stays under 1 quantum).

**Definition of Done**:
- [x] `VirtualTimeAuthority.step()` applies overshoot compensation on every call.
- [x] All 4 Python unit tests pass.
- [x] `make lint` (ruff) passes.
- [x] Existing phase-7 integration tests still pass (no regression; net drift over 50
  quanta must be < 1 quantum's worth of ns).

---

### **[ARCH-7] Fix `publisher.put().wait()` Under BQL** — Performance / Safety

**Status**: 🟡 Open. No dependencies.

**Goal**: All TX paths in `zenoh-chardev`, `zenoh-netdev`, and `zenoh-actuator` call
`publisher.put(payload).wait()` while holding the BQL. This blocks the QEMU main event
loop for the duration of the Zenoh network operation (10–50 µs). Replace with a
fire-and-forget pattern: push to a lock-free channel; a background sender thread drains
the channel without holding the BQL.

**Pattern** (mirror the SafeSubscriber approach, but for TX):
```rust
pub struct SafePublisher {
    tx: crossbeam_channel::Sender<Vec<u8>>,
    is_valid: Arc<AtomicBool>,
    sender_thread: Option<JoinHandle<()>>,
}

impl SafePublisher {
    pub fn new(publisher: Publisher<'static>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let is_valid = Arc::new(AtomicBool::new(true));
        let valid = Arc::clone(&is_valid);
        let sender_thread = std::thread::spawn(move || {
            while let Ok(payload) = rx.recv() {
                if !valid.load(Ordering::Acquire) { break; }
                let _ = publisher.put(payload).wait();
            }
        });
        Self { tx, is_valid, sender_thread: Some(sender_thread) }
    }

    /// Called from MMIO handler (BQL held). Non-blocking.
    pub fn send(&self, payload: Vec<u8>) {
        if self.is_valid.load(Ordering::Acquire) {
            let _ = self.tx.send(payload);
        }
    }
}

impl Drop for SafePublisher {
    fn drop(&mut self) {
        self.is_valid.store(false, Ordering::Release);
        // Dropping tx closes the channel, which unblocks recv() in sender_thread.
        // The sender_thread then exits naturally.
        if let Some(t) = self.sender_thread.take() { let _ = t.join(); }
    }
}
```

**Files to create**:
- `hw/rust/virtmcu-zenoh/src/publisher.rs` — `SafePublisher` struct

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs` — re-export `SafePublisher`
- `hw/rust/zenoh-chardev/src/lib.rs` — replace `publisher.put().wait()` with `safe_pub.send()`
- `hw/rust/zenoh-netdev/src/lib.rs` — same
- `hw/rust/zenoh-actuator/src/lib.rs` — same (if applicable)

**Unit tests** (in `publisher.rs`):
- `test_safe_publisher_sends_payload`: create `SafePublisher` with a mock publisher that
  logs payloads; call `send(b"hello")`, wait 50ms, assert mock received `b"hello"`.
- `test_safe_publisher_non_blocking_under_load`: measure time for 1000 `send()` calls;
  assert total wall-clock < 1ms (send must not block even if publisher is slow).
- `test_safe_publisher_drop_joins_thread`: drop `SafePublisher`; assert drop completes
  within 500ms (sender thread exits cleanly).

**Definition of Done**:
- [ ] `SafePublisher` exists in `hw/rust/virtmcu-zenoh/src/publisher.rs`.
- [ ] `grep -r "publisher.put.*wait" hw/rust/zenoh-chardev/ hw/rust/zenoh-netdev/` finds
  zero matches.
- [ ] All 3 unit tests pass.
- [ ] MMIO handler TX latency < 1µs (assert in integration test via `perf` or timing
  measurement).
- [ ] `make lint` passes.

---

### **[ARCH-8] TA/Coordinator Synchronization Protocol** — Correctness

**Status**: 🟡 Open. Depends on: DET-5 (coordinator) and DET-4 (Unix clock transport).

**Goal**: There is a race: the TimeAuthority can send quantum Q+1's clock advance to
QEMU nodes *before* the coordinator has finished delivering quantum Q's messages. This
allows Q+1 firmware execution to begin before it sees Q's messages — violating causal
ordering.

**Chosen solution (coordinator-as-gatekeeper)**:

The coordinator controls the `sim/clock/start/{node_id}` topic. QEMU's clock transport
does not directly receive the advance reply from the TA. Instead:
1. TA sends advance requests to all nodes (as before).
2. Coordinator, upon completing a quantum's delivery, publishes to `sim/clock/start/{node_id}`.
3. `UnixSocketClockTransport` / `ZenohClockTransport` only sends the advance reply to QEMU
   after receiving the coordinator's `start` signal.

The TA is still the timing master; the coordinator only gates the *release* of the reply.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — add subscription to `sim/clock/start/{node_id}`;
  `recv_advance` blocks until both TA reply AND coordinator start signal are received
- `tools/deterministic_coordinator/src/main.rs` — after delivery of Q's messages, publish
  `sim/clock/start/{node_id}` to all nodes
- `docs/design/COORDINATOR_SYNC_PROTOCOL.md` — write the formal protocol spec (new doc)

**Formal protocol** (write in `COORDINATOR_SYNC_PROTOCOL.md`):
```
Step 1: TA sends ClockAdvanceReq to each node via ClockSyncTransport.
Step 2: Each node executes quantum Q (firmware runs).
Step 3: Each node sends all outbound messages + "done" signal to coordinator.
Step 4: Coordinator waits for all N nodes to signal done.
Step 5: Coordinator sorts, masks, and delivers all messages.
Step 6: Coordinator publishes sim/clock/start/{node_id} to ALL nodes.
Step 7: Each node releases the ClockReadyResp to the TA.
Step 8: TA receives all replies and proceeds to quantum Q+1.
```

**Unit tests**:
- `test_no_q1_before_delivery`: mock coordinator that delays `start` signal by 50ms;
  assert firmware on node B does not see node A's Q messages before the `start` fires.

**Integration test** (`tests/test_arch8_coordinator_sync.py`):
- 2-node simulation; node A sends a message in quantum 5; assert node B's firmware
  observes the message in quantum 5 context (not prematurely in quantum 4).
- Deliberately inject a 100ms delay in coordinator delivery; assert clock does not advance
  until delivery completes.

**Definition of Done**:
- [ ] `COORDINATOR_SYNC_PROTOCOL.md` written with the 8-step protocol above.
- [ ] No Q+1 advance released before Q delivery is complete (verified by integration test).
- [ ] `make lint` passes.

---

### **[ARCH-9] Unbounded Backlog Admission Control** — Reliability

**Status**: 🟡 Open. Depends on: ARCH-5 for the coordinator side.

**Goal**: The priority-queue backlog in `zenoh-chardev` and `zenoh-netdev` can grow
without bound if firmware consumes incoming frames slower than they arrive. Under flood
conditions this exhausts QEMU process memory. Add a drop-tail policy.

**Files to modify**:
- `hw/rust/zenoh-chardev/src/lib.rs` — add `max_backlog: usize` property (default 256);
  in subscriber callback, if `backlog.len() >= max_backlog`, drop the new frame and
  increment `dropped_frames: AtomicU64`
- `hw/rust/zenoh-netdev/src/lib.rs` — same pattern

**QEMU property**:
```
-device zenoh-chardev,...,max-backlog=512
```

**Unit tests** (Rust):
- `test_backlog_drops_at_limit`: push `max_backlog + 1` frames into the backlog; assert
  the backlog size is exactly `max_backlog` and `dropped_frames == 1`.
- `test_backlog_accepts_up_to_limit`: push exactly `max_backlog` frames; assert all are
  accepted and `dropped_frames == 0`.

**Definition of Done**:
- [ ] `max-backlog` property exists with default 256.
- [ ] `dropped_frames` counter is observable via QOM property read.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

### **[ARCH-10] Zenoh Session Watchdog** — Reliability

**Status**: 🟡 Open. No dependencies.

**Goal**: If the Zenoh router dies mid-simulation, `zenoh-clock` spins forever in
`recv_advance` with no diagnostics. Add a watchdog: if no advance arrives within
`3 × stall_timeout_ms`, enter a `SessionLost` state, log a fatal error, and cleanly
exit QEMU.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — add `session_watchdog_ms` property (default:
  `3 × stall_timeout_ms`); in `recv_advance` timeout path, increment a miss counter;
  when miss count exceeds threshold, call `cpu_abort()` with a clear message

**Implementation sketch**:
```rust
// In recv_advance timeout path (ClockSyncTransport → ZenohClockTransport):
self.consecutive_timeouts.fetch_add(1, Ordering::Relaxed);
if self.consecutive_timeouts.load(Ordering::Relaxed) > self.watchdog_threshold {
    // Log to QEMU monitor and abort cleanly.
    virtmcu_qom::vlog!(
        "[zenoh-clock] FATAL: No clock advance received in {} consecutive timeouts. \
         Zenoh router may be down. Aborting.\n",
        self.watchdog_threshold
    );
    // Exit QEMU cleanly so the Python test harness can detect the failure.
    std::process::exit(1);
}
```

Reset `consecutive_timeouts` to 0 on every successful `recv_advance`.

**Unit tests**:
- `test_watchdog_triggers_after_threshold`: use `MockClockTransport` configured to timeout
  on all calls; assert that after `watchdog_threshold` calls, the watchdog fires (capture
  via a mock `abort_fn` hook rather than actual process exit).
- `test_watchdog_reset_on_success`: one success after N-1 timeouts; assert `consecutive_timeouts == 0`.

**Definition of Done**:
- [ ] Watchdog property `session-watchdog-ms` configurable via QEMU device property.
- [ ] QEMU exits cleanly (non-zero code) when watchdog fires.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

### **[ARCH-11] Generation Counter for `SafeSubscriber` Mid-Lifetime Validity** — Correctness

**Status**: 🟡 Open. Depends on: DET-1 complete.

**Goal**: When a QOM device is *reset* (not finalized — firmware reboots the MCU),
`SafeSubscriber` remains live but the device's internal state is re-initialized. A
callback that was queued before the reset may inject stale data into the fresh state.
Add a generation counter: the device increments it on reset; the subscriber callback
drops the message if the generation does not match.

**Files to modify**:
- `hw/rust/virtmcu-zenoh/src/lib.rs` — add `generation: Arc<AtomicU64>` field to
  `SafeSubscriber`; check in callback before calling the inner callback
- Every peripheral using `SafeSubscriber` — pass `Arc<AtomicU64>` from the device
  state; increment it in the QOM `reset` handler

**API change**:
```rust
pub struct SafeSubscriber {
    // ... existing fields ...
    generation: Arc<AtomicU64>,
    expected_generation: AtomicU64,
}

impl SafeSubscriber {
    pub fn new<F>(
        session: &Session,
        topic: &str,
        generation: Arc<AtomicU64>,
        callback: F,
    ) -> Result<Self, zenoh::Error>
    where F: Fn(zenoh::sample::Sample) + Send + Sync + 'static { ... }
}
```

In the callback wrapper:
```rust
let current_gen = generation_clone.load(Ordering::Acquire);
let expected_gen = expected_gen_clone.load(Ordering::Acquire);
if current_gen != expected_gen {
    // Stale message from a previous device lifetime — discard.
    return;
}
```

**Unit tests**:
- `test_generation_drop_stale_callback`: create `SafeSubscriber` with generation=0;
  queue a message; increment generation to 1 before the message fires; assert callback
  is NOT invoked.
- `test_generation_accepts_current`: generation is 2; message queued with expected=2;
  assert callback IS invoked.

**Definition of Done**:
- [ ] `SafeSubscriber::new` takes `Arc<AtomicU64> generation` parameter.
- [ ] All peripherals pass their reset-generation counter.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

### **[ARCH-12] Replace Heartbeat Thread with Zenoh Liveliness Token** — Code Quality

**Status**: 🟡 Open. No dependencies.

**Goal**: Any heartbeat background thread in `zenoh-clock` (or other peripherals) that
periodically publishes a "I am alive" message is unreliable — if the thread sleeps for
5 s and QEMU crashes, peers do not know for 5 s. Zenoh's built-in `Liveliness` token
is declared once per session and disappears *immediately* when the session closes, giving
instant health signaling with zero polling.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — remove heartbeat thread; declare
  `session.liveliness().declare_token("sim/node/{node_id}/alive")`; store the token in
  `ZenohClockBackend` to keep it alive
- `tests/conftest.py` or `tests/time_authority.py` — replace heartbeat subscription with
  Zenoh Liveliness subscriber for node health monitoring

**Implementation**:
```rust
let liveness_token = session
    .liveliness()
    .declare_token(format!("sim/node/{}/alive", node_id))
    .wait()
    .map_err(|e| format!("liveliness token failed: {e}"))?;
// Store token in backend to keep it alive for the session duration.
backend.liveness_token = Some(liveness_token);
```

**Unit tests** (Python, in `tests/test_arch12_liveliness.py`):
- `test_liveliness_token_visible_during_session`: start QEMU with zenoh-clock; use a
  Python Zenoh subscriber to `liveliness().get("sim/node/0/alive")`; assert the token
  is present.
- `test_liveliness_token_disappears_on_qemu_exit`: kill QEMU; assert the token disappears
  within 2 s (use liveliness subscriber's `Subscriber::subscriber_alive` event).

**Definition of Done**:
- [ ] No heartbeat `thread::spawn` in `zenoh-clock`.
- [ ] Liveliness token declared and stored.
- [ ] Both Python tests pass.
- [ ] `grep -n "heartbeat" hw/rust/zenoh-clock/src/lib.rs` finds zero matches.
- [ ] `make lint` passes.

---

### **[ARCH-13] Clock Session Priority Isolation** — Performance

**Status**: 🟡 Open. Partially addressed by DET-4 (Unix socket). This task documents
and enforces the isolation regardless of transport.

**Goal**: When the clock transport is Zenoh (multi-host mode), the clock `GET` query
competes with high-volume emulated network traffic on the same shared Zenoh session.
A single burst of 1000 Ethernet frames can delay the clock reply by 10–50 ms, causing
spurious STALL events.

**Solution**: Use a *dedicated* Zenoh session for clock sync (separate from the data-plane
session from DET-2). The two sessions connect to the same router but have independent
executor thread pools, eliminating contention.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — do not use `get_or_init_session()`; instead call
  `open_session(router)` to get a private session
- `hw/rust/virtmcu-zenoh/src/lib.rs` — document in `get_or_init_session` doc comment that
  it is for the *data plane only* and clock must use its own session

**Unit tests** (integration, `tests/test_arch13_clock_isolation.py`):
- Boot a 2-node simulation. Flood the data plane with 10 000 frames from node A to B.
- Simultaneously measure clock advance RTT on node A.
- Assert median clock RTT < 5ms even during the flood (compare with DET-4 baseline).

**Definition of Done**:
- [ ] `zenoh-clock` does not call `get_or_init_session()` — uses its own private session.
- [ ] Comment in `get_or_init_session` states "data plane only; zenoh-clock uses dedicated session".
- [ ] Integration test passes: clock RTT < 5ms during 10 000-frame flood.
- [ ] `make lint` passes.

---

### **[ARCH-14] Document and Measure Simulation Frequency Ceiling** — Observability

**Status**: 🟡 Open. Depends on: DET-4 (Unix socket transport).

**Goal**: Document the maximum sustainable quantum rate for each transport option.
Add a benchmark script. Add the measured results as a table in ARCHITECTURE.md so
engineers can choose the right transport for their scenario.

**Files to create**:
- `tools/benchmarks/clock_rtt_bench.py` — measures median clock RTT across 10 000 quanta

**Files to modify**:
- `docs/design/ARCHITECTURE.md` — add "Simulation Frequency Ceiling" table

**Benchmark methodology**:
```python
# tools/benchmarks/clock_rtt_bench.py
# Usage: python3 clock_rtt_bench.py --transport [unix|zenoh] --quanta 10000
import statistics, time
rtts = []
for _ in range(args.quanta):
    t0 = time.perf_counter_ns()
    client.advance(delta_ns=1_000_000)   # 1 ms quantum
    rtts.append(time.perf_counter_ns() - t0)
print(f"Median RTT: {statistics.median(rtts)/1000:.1f} µs")
print(f"P99 RTT:    {sorted(rtts)[int(0.99*len(rtts))]/1000:.1f} µs")
print(f"Max freq:   {1e9/statistics.median(rtts):.0f} Hz")
```

**Expected results table** (to add to ARCHITECTURE.md §9 Performance):
| Transport | Median RTT | P99 RTT | Max quantum rate |
|---|---|---|---|
| Unix socket (same host) | ~2 µs | ~10 µs | ~500 kHz |
| Zenoh local router (same host) | ~20 µs | ~80 µs | ~50 kHz |
| Zenoh remote router (LAN) | ~200 µs | ~500 µs | ~5 kHz |

*(Fill actual measured values during implementation.)*

**Definition of Done**:
- [ ] Benchmark script exists and is runnable in CI.
- [ ] Results table added to ARCHITECTURE.md §9.
- [ ] `make lint` (ruff) passes on the benchmark script.

---

### **[ARCH-15] SMP Firmware Quantum Barrier** — Correctness for Dual-Core Firmware

**Status**: 🟡 Open. No dependencies. Low priority unless dual-core firmware is needed.

**Goal**: When QEMU is started with SMP (`-smp 2`), the TCG quantum hook fires
independently on each vCPU thread. The current model sends the `ClockReadyResp` after
the *first* vCPU hits the boundary, while the second may still be executing — allowing
a partial quantum where only half the firmware ran.

**Required model**: Both vCPUs must halt at the quantum boundary before any
`ClockReadyResp` is sent. Implement a per-quantum vCPU barrier counter.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — add `n_vcpus: u32` QOM property (default 1);
  add `vcpu_halt_count: AtomicU32`; in the quantum hook, increment the counter and wait
  (using `Condvar`) until `vcpu_halt_count == n_vcpus` before sending `ClockReadyResp`;
  reset counter at quantum start.

**Implementation sketch**:
```rust
// In the quantum hook (each vCPU calls this independently):
let count = backend.vcpu_halt_count.fetch_add(1, Ordering::AcqRel) + 1;
if count == backend.n_vcpus {
    // Last vCPU to halt — send the ready reply.
    backend.transport.send_ready(resp)?;
    backend.vcpu_halt_count.store(0, Ordering::Release);
    backend.all_vcpus_halted_cond.notify_all();
} else {
    // Wait for all vCPUs to halt before releasing.
    let mut guard = backend.vcpu_mutex.lock().unwrap();
    while backend.vcpu_halt_count.load(Ordering::Acquire) != backend.n_vcpus {
        guard = backend.all_vcpus_halted_cond.wait(guard).unwrap();
    }
}
```

**Unit tests**:
- `test_smp_barrier_waits_for_all_vcpus`: mock N=2 vCPUs; vCPU 0 calls hook first;
  assert no `ClockReadyResp` sent; vCPU 1 calls hook; assert reply sent.
- `test_smp_barrier_n1_behaves_as_before`: N=1; single call sends reply immediately.

**Definition of Done**:
- [ ] `n-vcpus` property added to `zenoh-clock` device.
- [ ] With `n-vcpus=2` and `-smp 2`, both vCPUs halt before reply is sent.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

### **[ARCH-16] Remove Misleading `#![no_std]` from Peripherals** — Code Quality

**Status**: 🟡 Open. No dependencies.

**Goal**: Several `hw/rust/` crates carry `#![no_std]` at the top of `lib.rs` but
transitively depend on `std` through `zenoh` (which uses `tokio`, `std::net`, etc.).
The annotation is misleading to readers and should be removed.

**Files to check** (run `grep -rl "no_std" hw/rust/ --include="*.rs"`):

For each file found:
1. If the crate genuinely uses no `std` features (no `Vec`, `String`, `Arc`, etc.) and
   compiles with `--target thumbv7m-none-eabi`, keep `#![no_std]` and document why.
2. Otherwise, remove the annotation and add a `// std is required: zenoh/tokio bring std`
   comment at the crate root's `lib.rs` if the change would surprise a reader.

**Verification**:
```bash
# This must fail to compile — confirming no_std was never genuinely in effect:
cargo check --target thumbv7m-none-eabi -p zenoh-clock 2>&1 | grep "error"
# If it compiles cleanly, the annotation was actually meaningful — do NOT remove it.
```

**Definition of Done**:
- [ ] `grep -rl "#!\[no_std\]" hw/rust/ --include="*.rs"` returns only crates that
  genuinely compile without std (verified by the thumbv7m check above).
- [ ] `make lint` passes.

---

### **[ARCH-17] Replace `GLOBAL_CLOCK` Singleton to Support Multi-MCU QEMU** — Architecture

**Status**: 🟡 Open. Low priority. Depends on: ARCH-1 and ARCH-3 complete.

**Goal**: `GLOBAL_CLOCK` is a process-wide singleton. If a user instantiates two
`zenoh-clock` devices in one QEMU process (e.g., a dual-MCU board), the second
instantiation overwrites the first. Replace with a per-device-instance registry keyed by
node ID, allowing multiple independent clock devices per QEMU process.

**Files to modify**:
- `hw/rust/zenoh-clock/src/lib.rs` — replace `static GLOBAL_CLOCK: AtomicPtr<ZenohClock>`
  with `static CLOCK_REGISTRY: Mutex<HashMap<u32, Arc<ZenohClock>>>` (keyed by `node_id`)
- The TCG hook must look up the clock by the calling vCPU's node association (passed as
  the `opaque` parameter in the hook function pointer)

**Unit tests**:
- `test_two_clock_instances_independent`: instantiate two `ZenohClockBackend` objects
  (different node IDs); assert each receives independent `MockClockTransport` advances;
  assert no cross-contamination.

**Definition of Done**:
- [ ] `GLOBAL_CLOCK: AtomicPtr` removed.
- [ ] `CLOCK_REGISTRY: Mutex<HashMap<u32, Arc<ZenohClock>>>` introduced.
- [ ] `test_two_clock_instances_independent` passes.
- [ ] `make lint` passes.

---

### **[ARCH-18] Formal Quantum Number Alignment** — Protocol Correctness

**Status**: 🟡 Open. Depends on: DET-5 (coordinator) and DET-3 (ClockSyncTransport).

**Goal**: The TimeAuthority increments a `quantum_number` counter; the
`DeterministicCoordinator` also tracks a `quantum_number`. Currently these are not
exchanged in the wire protocol — a restart or reconnect can leave them out of sync,
causing the coordinator to deliver Q's messages to nodes that have already moved to Q+1.

**Wire protocol change**: Add `quantum_number: u64` to both `ClockAdvanceReq` and the
coordinator's `done` signal. The TA, QEMU, and coordinator must all agree on the current
quantum number. Reject messages with mismatched quantum numbers.

**Files to modify**:
- `hw/rust/virtmcu-api/src/lib.rs` — add `quantum_number: u64` to `ClockAdvanceReq`
  and `ClockReadyResp` (both sides echo the number)
- `hw/rust/zenoh-clock/src/lib.rs` — echo `quantum_number` in `ClockReadyResp`
- `tools/deterministic_coordinator/src/barrier.rs` — include `quantum_number` in the
  `done` signal; reject `submit_done` calls where the quantum number does not match the
  expected current quantum
- `tests/time_authority.py` — TA tracks and sends `quantum_number`

**Unit tests**:
- `test_quantum_number_echoed_in_reply`: send advance with `quantum_number=42`; assert
  the `ClockReadyResp` echoes `quantum_number=42`.
- `test_coordinator_rejects_wrong_quantum`: coordinator in quantum 5 receives a `done`
  signal with `quantum_number=4`; assert the signal is rejected with an error log.

**Definition of Done**:
- [ ] `quantum_number` field in both `ClockAdvanceReq` and `ClockReadyResp`.
- [ ] Coordinator validates quantum numbers on every `done` signal.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

## 5. Ongoing Risks (Watch List)

Items here have no immediate action — they are structural constraints or future triggers to monitor.

| ID | Risk | Status / Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Ongoing. QEMU version is pinned; all patches go through `scripts/apply-qemu-patches.sh`. Track upstream `accel/tcg` changes on each QEMU bump. |
| R7 | `icount` performance | Design guideline: use `slaved-icount` only when sub-quantum timing precision is required (PWM, µs-level). `slaved-suspend` is the default. |
| R11 | Zenoh session deadlocks in teardown | Partially mitigated: `SafeSubscriber` calls `undeclare().wait()` in `drop()`. Remaining risk: calling `.wait()` from inside a Zenoh callback context can still deadlock. Monitor for new peripherals that add Zenoh callbacks with complex teardown. |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov`/coverage CI gate to prove peripherals exercise firmware code paths. Tracked as Phase 30.8. |
| R20 | `remote-port` endianness assumption | Fixed: `bridge_write` uses `to_le_bytes()`, read-back uses `from_le_bytes()`. Assumption holds for all current x86/ARM hosts. If a BE host is ever added, `DEVICE_NATIVE_ENDIAN` must become `DEVICE_LITTLE_ENDIAN`. |
| R21 | PDES tie-breaking gap (pre-DET-5) | Until DET-5 ships, same-virtual-time messages between nodes are delivered in OS-scheduler order — non-deterministic. Mitigation: existing tests use single-node or strictly ordered topologies. Risk materialises only in multi-node tests with concurrent same-vtime messages. |
| R22 | `SafeSubscriber` UAF window (pre-DET-1) | The bounded spinloop in `drop()` can exit before callbacks finish under ASan/TSan load. Risk of UAF is low in production (callbacks are short) but will fail under sanitizers. Fix is tracked as DET-1 (P0). |
| R23 | Zenoh executor as SPOF for clock | Until DET-4 ships, all clock-sync RTT goes through the Zenoh router. Router restart hangs all nodes. Mitigation: CI always starts the router before QEMU; production uses systemd restart policies. |

---

## 6. Permanently Rejected / Won't Do
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
