# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation.
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
- **The Beyonce Rule**: "If you liked it, you shoulda put a test on it." Prove bugs with failing tests before fixing.
- **Lint Gate**: `make lint` must pass before every commit (ruff, version checks, cargo clippy).

---

## 1. P0: Holistic Eradication of Non-Deterministic Sleeps (COMPLETED)

### **SYSTEMIC HARDENING: Total Determinism & "Zero-Sleep" Mandate**
**Status**: ✅ Completed. Core `tests/` directory and all Rust plugins have been refactored for enterprise-grade determinism.

#### **Pattern 1: Zenoh Discovery (The "Network Mesh" Problem)**
- **Enterprise Fix**: Implemented `wait_for_zenoh_discovery` in `conftest.py`. It explicitly verifies connectivity rather than relying on arbitrary `sleep(2.0)` delays. 

#### **Pattern 2: Socket/Service Readiness (The "Connection Refused" Problem)**
- **Enterprise Fix**: Hardened Pytest setup loops and legacy Bash scripts (like `tcp_router_test.sh`) with deterministic `socket.create_connection` or `ss -tln` polling loops.

#### **Pattern 3: Firmware/Guest Boot (The "Is it Ready?" Problem)**
- **Enterprise Fix**: Successfully migrated Phase 7, 8, 12, and Actuator tests to use `VirtualTimeAuthority` (for `slaved-icount` progression) and `wait_for_line_on_uart()` (for deterministic readiness signals).

#### **Pattern 4: Data Pacing (The "Overflow" Problem)**
- **Enterprise Fix**: Successfully implemented proper flow control (backlog `VecDeque` + backend `can_receive` checks + drain callbacks) across all major plugins: `zenoh-chardev`, `zenoh-netdev`, and `zenoh-canfd`. Stress tests now run without artificial `time.sleep` pacing.

---

## 2. P0: Immediate Actions (Ordered by Dependency)

> **Last updated**: 2026-04-25 after full audit of Gemini's Tasks A–G (commit `9d84f98`).
> Tasks are sequenced. A task marked **[UNBLOCKED]** can start immediately. A task marked **[NEEDS Pxx]** must wait for that task to finish. Complete each task with a passing `make lint` + stress test before moving to the next.
> **Mandatory before every commit**: `make lint && make test-unit` must both pass.

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

**Verification required**:
1. Running `python3 scripts/gen_vproto.py` regenerates `tools/vproto.py` byte-for-byte identically to the committed version (idempotent).
2. `make lint` fails if a field is added to a Rust struct but `gen_vproto.py` is not re-run.
3. All existing `test_vproto.py` tests still pass.
4. The generated constants (`VIRTMCU_PROTO_MAGIC`, `VIRTMCU_PROTO_VERSION`, `MMIO_REQ_READ`, etc.) are also extracted from the Rust source constants, not hardcoded in the generator.

**Scope boundary**: Do NOT introduce FlatBuffers in this task. FlatBuffers is appropriate for higher-level Zenoh messages (telemetry, FlexRay) where it is already established. The `mmio-socket-bridge` and clock protocols use fixed-frame binary formats that are shared with external C++ tooling — changing their wire format requires a coordinated multi-repository migration.

---

### **[P00 — CLOSED ✅ 2026-04-25] Re-enable Phase 7 Tests**
- **Fixed**: Removed skips and stabilized Phase 7 tests.
- **Root Cause**: Tests timed out because ASan QEMU boot takes >5s, triggering false `STALL DETECTED` on the initial `vta.step(0)`. We explicitly increased `stall-timeout=60000` for these tests. Fixed `pytest-timeout` hangs during test teardown by correctly async-awaiting `start_emulation()` in finally blocks to ensure the vCPU thread unblocks and gracefully exits QEMU.
- **Verified**: 100% pass rate over `pytest tests/test_phase7.py -n auto -v` x3.

### **[P01 — OPEN] Robust ASan Boot-Time Stall Timeout Mechanism**
**Goal**: Fix the underlying architectural flaw where `zenoh-clock` uniformly enforces a strict `stall-timeout` (default 5s) even on the very first quantum, causing spurious timeouts under ASan without manual hacks.
- **Problem**: ASan overhead causes TCG initialization to take 10-20 seconds. The first `vta.step(0)` triggers a timeout, forcing test authors to manually add `stall-timeout=60000` to QEMU CLI arguments, masking genuine deadlocks that might occur later in the test.
- **Steps**:
  1. Add `is_first_quantum: AtomicBool` to `ZenohClockBackend`.
  2. In `process_clock_query`, if `is_first_quantum` is true, grant a generously scaled timeout (e.g., 60-120 seconds or `VIRTMCU_STALL_TIMEOUT_MS`). Once the quantum completes, clear the flag.
  3. Ensure subsequent quanta fall back to the strict `stall_timeout_ms` (e.g., 5 seconds).
  4. Write an integration test asserting this "slow boot, fast execute" timeout behavior.
  5. Remove the `stall-timeout=60000` hacks from `tests/test_phase7.py` and `tests/test_phase8.py`.

---

### **[P01 — PARTIALLY CLOSED]** Fix Main Branch Test Failures *(280/290 passing as of 2026-04-24)*
**Goal**: Restore the `main` branch to a fully green state before any refactoring.
**Why first**: All subsequent P0 work will generate commits on `main`. A green baseline is required to detect regressions introduced by each change.
**Definition of Done**: `make ci-full` exits 0 inside the devcontainer on a clean clone, and the GitHub Actions pipeline shows all jobs green for two consecutive runs.
1. Run `gh run list --limit 5` and `gh run view --log` to enumerate every failing test by name.
2. Run each failing test in isolation (`pytest tests/test_foo.py -v`) and then under load (`pytest tests/test_foo.py -n auto --count=20`) to distinguish flakes from deterministic failures.
3. Apply surgical fixes to restore stability.
4. Once individually passing, run the full suite 3 times end-to-end. Target: zero failures across all runs.

---

### **[P02 — CLOSED ✅ 2026-04-24] Fix Unaligned Packed Struct Read UB in `remote-port`**
- Fixed: all `#[repr(C, packed)]` reads from byte buffers now use `ptr::read_unaligned`.
- Verified: `test_unaligned_hdr_read`, `test_unaligned_busaccess_read`, `test_unaligned_interrupt_read`.

---

### **[P03 — CLOSED ✅ 2026-04-24] Upgrade `sync.rs` Mock & Add Unit Tests**
**Goal**: The BQL abstraction layer must have tested invariants before any peripheral refactoring relies on it.
- **Problem**: The test mock in `sync.rs` returns `1` (signaled) unconditionally from `virtmcu_cond_timedwait`, making timeout paths permanently invisible to unit tests.
- **Steps**:
  1. Upgrade the mock to use a `thread_local` or per-test state that supports configurable return values (signaled vs. timed-out).
  2. Write `#[test]` unit tests covering: `BqlGuard` drop releases lock; `BqlUnlockGuard` re-acquires on drop; `temporary_unlock()` returns `None` when BQL is not held; `wait_timeout` timeout path (mock returns 0); `wait_timeout` signal path (mock returns 1).
- **Why before P04**: P04 depends on `wait_yielding_bql` being tested and correct before it is used in peripherals.

---

### **[P04 — CLOSED ✅ 2026-04-25] Enterprise BQL Safety: Audit & Eliminate All Direct FFI Calls**

**Status**: ✅ CLOSED. The audit and migration were completed across all Rust peripherals.

#### **Completed Work**:
1. **Mechanical Audit**: Replaced all direct `virtmcu_bql_locked()` FFI calls with `virtmcu_qom::sync::Bql::is_held()`.
2. **BqlGuarded Migration**: Introduced `BqlGuarded<T>` to replace `Mutex<T>` for state protected by BQL. Migrated `zenoh-802154`, `zenoh-netdev`, `zenoh-flexray`, `zenoh-telemetry`, `zenoh-ui`, `zenoh-canfd`, and `zenoh-chardev`.
3. **Lint Enforcement**: Added `make lint` gate banning `Mutex<T>` in peripheral state structs.
4. **Verification**: `make lint` passes and zero direct FFI calls to BQL/Mutex primitives remain in peripheral code.
**Goal**: Provide the one approved way to block a vCPU thread on a CondVar. Eliminate all direct FFI calls to `virtmcu_mutex_lock/unlock` from peripheral code.
- **What**: Implement `QemuCond::wait_yielding_bql<'a>(&mut self, guard: QemuMutexGuard<'a>, timeout_ms: u32) -> (QemuMutexGuard<'a>, bool)` in `virtmcu-qom/src/sync.rs`. **Full contract**: caller passes ownership of a `QemuMutexGuard` (proving the peripheral mutex is locked). The function: (1) releases the BQL via `Bql::temporary_unlock()`; (2) calls `wait_timeout` on the CondVar; (3) re-acquires BQL (BqlUnlockGuard drop); (4) returns the guard and a bool (true = signaled, false = timed out). The peripheral mutex is held on both entry and exit.
- **Also implement**: `QemuCond::wait_yielding_bql_loop` — same contract but loops until a predicate returns true or a timeout expires. This eliminates the while-loop boilerplate in every bridge.
- **Audit**: After implementing, grep the entire `hw/rust/` tree for direct calls to `virtmcu_mutex_lock`, `virtmcu_mutex_unlock`, `virtmcu_bql_lock`, `virtmcu_bql_unlock` outside of `virtmcu-qom/src/sync.rs`. Each must be eliminated.
- **BANNED after this task**: Direct FFI calls to BQL or peripheral mutex primitives from any peripheral crate.
- **What can go wrong**: AB-BA deadlocks during refactoring. The new abstraction must guarantee BQL is re-acquired in all code paths including panics — use `scopeguard` or a drop impl.

---

### **[P05 — CLOSED ✅ 2026-04-24] Dual Locking Scheme Consolidation**
- Both `mmio-socket-bridge` and `remote-port` now use pure Rust `std::sync::Mutex`/`Condvar`. Raw `*mut QemuMutex`/`*mut QemuCond` eliminated. Lock order documented in module-level comments.

---

### **[P06 — CLOSED ✅ 2026-04-24] Fix Device Teardown UAF — Shutdown Safety**
- `VcpuCountGuard` RAII ensures `active_vcpu_count` is decremented even on panic. `drain_cond.wait_timeout(30 s)` replaces the bounded spinloop. Applied to both bridges.

---

### **[P07 — CLOSED ✅ 2026-04-24] Eradication of `std::thread::sleep` in `hw/rust/`**
- All reconnect/connection-wait sleeps replaced with `connected_cond.wait_timeout()`. Heartbeat loop uses `backend.cond.wait_timeout(1s)`. Only `SLEEP_EXCEPTION`-tagged sleeps remain in `virtmcu-zenoh`. CI grep gate in `make lint-rust` enforces zero untagged sleeps.

---

### **[P08 — NEEDS P06] ASan in Continuous PR Gate**
**Goal**: Address Sanitizer catches UAF bugs that normal testing misses. Must run on every PR.
- **Why after P06**: Running ASan before the teardown UAF is fixed would produce noise. Fix the known bugs first, then gate on ASan to prevent regressions.
- **Steps**:
  1. Add `make test-asan` target: `RUSTFLAGS="-Z sanitizer=address" cargo +nightly test --workspace`.
  2. Add GitHub Actions job using `devenv-base` container (already has nightly Rust).
  3. Gate PR merge on this job.
- **Scope**: Rust unit tests (`cargo test`) is the minimum bar. Full QEMU-level ASan is Phase 30.

---

### **[P09 — NEEDS P04] Eliminate `#[allow(...)]` Lint Suppressors & `static mut` Properties**
**Goal**: Zero `#[allow(...)]` in production code. Enforce via `cargo clippy -- -D warnings`.
- **Current violations**:
  - `zenoh-clock/src/lib.rs`: `#[allow(clippy::too_many_lines)]` × 2 — split the functions.
  - `mmio-socket-bridge`, `remote-port`, and all other peripherals: `#[allow(static_mut_refs)]` — caused by `static mut BRIDGE_PROPERTIES`.
- **Fix for `static mut BRIDGE_PROPERTIES`**: Replace with a safe static pattern. Evaluate whether `Property` fields are `const`-constructible (preferred — zero overhead); if not, use `OnceLock`. Apply consistently to all peripherals.
- **CI enforcement**: Update `make lint-rust` to pass `-- -D warnings` to `cargo clippy`. This makes every suppressor a build failure.

---

### **[P10 — UNBLOCKED, parallel with P03–P09] Enterprise-Grade Simulation & Testing Hardening**

#### **Part 1: Fix `zenoh-chardev` Flow Control (Core Bug)**
- **Issue**: `qemu_chr_be_write` called without checking `qemu_chr_be_can_write` — overflows PL011's 32-byte FIFO, causing data corruption.
- **Steps**: Add backpressure via a ring-buffer and `chr_accept_input` drain callback.
- **Verification**: "Burst Test" in `test_phase8.py` — 128-byte single-packet send must arrive uncorrupted in both `standalone` and `slaved-icount` modes.

#### **Part 2: Enterprise Framework Improvements (`conftest.py`)**
- **Part 2.1: Deterministic Zenoh Discovery Gates**: Implement `wait_for_zenoh_discovery(session, topic, count)` using Zenoh's Liveliness API (not the REST plugin — it is not guaranteed to be enabled in all router configurations). Use a configurable timeout with a diagnostic dump on failure.
- **Part 2.2: Centralized `VirtualTimeAuthority` Fixture**: `time_authority.run_until(vtime)` and `time_authority.step(delta_ns)`. Auto-detect stalls and dump QMP CPU state.
- **Verification**: Port `test_phase6.py` and `test_phase8.py` to the new fixture.

#### **Part 3: Robust Phase 8 UART Overhaul**
- Restore `slaved-icount` as the default for all Zenoh UART tests.
- Use the "Marker Packet" pattern for topology drop tests (P1 with `drop=1.0`, then P2 as marker; P2 received but P1 not = drop proven).
- **Verification**: `pytest tests/test_phase8.py -n auto` × 100 runs. Target: 0 failures.

---

### **[P11 — COMPLETED] Eliminating Hardcoded Resources for Parallel Execution**
**Status**: ✅ Completed. Dynamic ports, `tmp_path` isolation, workspace-scoped cleanup, binary resolution all done.

---

### **[P12 — FUTURE] Deterministic Deadlock Detection (Virtual Time Budgets)**
**Goal**: Replace wall-clock timeouts for simulation goals with deterministic virtual-time budgets.
- **Problem**: We currently rely on a globally scaled wall-clock watchdog (300s under ASan) to catch deadlocks. This is slow and non-deterministic.
- **Implementation**:
  1. Define a `max_vtime_ns` for every test scenario (e.g., "Firmware echo must respond within 50ms virtual time").
  2. In the Python test orchestrator, check `vta.current_vtime` after every `step()`.
  3. Raise an immediate `FirmwareDeadlockError` if the budget is exceeded without achieving the test goal.
- **Prerequisites (What to do first)**:
  1. **Complete [P10 Part 2]**: Finalize the centralized `VirtualTimeAuthority` fixture in `conftest.py`. We need a single source of truth for virtual time before we can budget it.
  2. **Migration**: Port all remaining legacy bash smoke tests (`test/phase*/smoke_test.sh`) to the Python `pytest` framework, as bash cannot easily track cumulative virtual time.
  3. **Phase 29 (Time Fidelity)**: Peripherals must implement realistic timing (baud rates/FIFOs). Without this, virtual time is "instant," and budgets are meaningless.
  4. **Establish Baselines**: Run the full suite with `VIRTMCU_USE_ASAN=0` to record "Golden" virtual-time durations for all boot and I/O sequences to use as budget limits.

---

### Restore Full Parallel Execution
**Goal**: Enable `pytest -n auto` without resource contention.
1. **Dynamic Resource Allocation**: Ensure UNIX sockets (QMP, UART) and Zenoh topics use dynamic ports/UUIDs.
2. **Artifact Isolation**: Use `tmp_path` for all generated DTBs, ELFs, and linker scripts.
3. **Zenoh Topic Isolation**: Use unique UUID prefixes for *every* test run.
4. **Remove `xdist_group(name="serial")`**: Once stable, remove all serial markers.
**Goal**: Enable `pytest -n auto` without resource contention.
1. **Dynamic Resource Allocation**: Ensure UNIX sockets (QMP, UART) and Zenoh topics use dynamic ports/UUIDs.
2. **Artifact Isolation**: Use `tmp_path` for all generated DTBs, ELFs, and linker scripts.
3. **Zenoh Topic Isolation**: Use unique UUID prefixes for *every* test run.
4. **Remove `xdist_group(name="serial")`**: Once stable, remove all serial markers.

---

## 3. Active Roadmap (Dependency Order)

### [Core] Phase 3.5 — YAML Platform Description & OpenUSD 🚧
*Depends on: Phase 3 (Parser) ✅*
- [ ] Complete YAML schema validation for all current peripherals.
- [ ] Ensure `yaml2qemu.py` supports new `zenoh-chardev` and `mmio-socket-bridge` mappings.

### [Core] Phase 4 — Robot Framework & QMP Hardening 🚧
*Depends on: Phase 1 (QEMU) ✅*
- [ ] Harden `QmpBridge` for high-latency or high-load scenarios.
- [ ] Ensure virtual-time-aware timeouts are used in all integration tests.

### [Core] Phase 6 & 7 — Deterministic Multi-Node Loop 🚧
*Depends on: Phase 1 (QEMU) ✅, Phase 18 (Rust Zenoh) ✅*
- [ ] **6.5** Multi-Node Ethernet Verification (Zephyr echo samples).
- [ ] **6.6** Industry-Standard Ethernet MAC Emulation (ADR-006).
- [ ] **7.8** Finalize `zenoh-netdev` RX determinism with priority queues.

### [Hardware] Phase 20.5 — SPI Bus & Peripherals 🚧
*Depends on: Phase 19 (Rust QOM) ✅*
- [ ] **20.5.1** SSI/SPI Safe Rust Bindings in `virtmcu-qom`.
- [ ] **20.5.2** Verify PL022 (PrimeCell) SPI controller in `arm-generic-fdt`.
- [ ] **20.5.3** Implement `hw/rust/zenoh-spi` bridge.
- [ ] **20.5.4** SPI Loopback/Echo Firmware verification.

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
*Depends on: Core synchronization (Phase 18)*
*Goal: Implement Software-Observable Fidelity (Option C from `PERIPHERAL_TIMING_EVALUATION.md`) to throttle immediate MMIO execution to physical baud rates using QEMUTimer.*
- [ ] **29.1** **FIFO & Timer Baseline**: Add TX/RX FIFO drain modeling using `QEMUTimer` to the `rust-dummy` peripheral template, including correct reset/teardown cancellation logic.
- [ ] **29.2** **UART Backpressure**: Upgrade `zenoh-chardev` and `s32k144-lpuart` to throttle TX interrupts based on configured baud rates, eliminating the "virtual time flooding" bug.
- [ ] **29.3** **RX Propagation Modeling**: Modify Zenoh subscribers in communication peripherals to queue incoming frames and use timers to simulate reception delay before asserting RX interrupts.
- [ ] **29.4** **Radio Delays (802.15.4)**: Implement CSMA/CA backoff timers and packet air-time modeling in the radio peripheral.
- [ ] **29.5** **Lifecycle Assertions**: Add test cases specifically asserting that disabling a peripheral or triggering a soft reset correctly calls `virtmcu_timer_del`, proving immunity to spurious IRQs caused by cancelled transmissions.

### [Infrastructure] Phase 30 — Deep Oxidization & Testing Overhaul 🚧
*Ongoing*
- [x] **30.6** Migrate `remote-port` to Rust.
- [ ] **30.8** Comprehensive Firmware Coverage (drcov integration).
- [ ] **30.9** Migrate `tools/systemc_adapter/` to Rust.
  - **What**: Rewrite `tools/systemc_adapter/main.cpp` (662 lines) and `remote_port_adapter.cpp` (96 lines) as a native Rust binary in `tools/rust/systemc-adapter/`.
  - **Why**: The adapter is a live simulation-path process handling concurrent Unix socket I/O, Zenoh pub/sub (clock advance + IRQ signaling), and the Remote Port protocol — exactly the threat model Rust is designed for. A Rust rewrite eliminates the last meaningful C++ production code outside of `third_party/` and `ffi.c`, and shares the already-existing `virtmcu-api` protocol types directly with no FFI.
  - **Depends on**: Phase 30.6 ✅ (`remote-port` Rust implementation documents the peer protocol). `virtmcu-api` ✅ (protocol types already in Rust).
  - **Steps**:
    1. Create `tools/rust/systemc-adapter/` crate in the Cargo workspace.
    2. Implement the Remote Port handshake, MMIO read/write dispatch, and IRQ signaling using `virtmcu-api` types and `zenoh` directly — no new protocol code, just a port.
    3. Replace SystemC TLM socket with the Rust `UnixListener` + async (or sync threaded) accept loop.
    4. Add `make build-systemc-adapter` target. Update CI to build and test it.
    5. Add a smoke test in `tests/` that wires the Rust adapter to a `mmio-socket-bridge` device and verifies a round-trip MMIO read.
    6. Deprecate and remove the C++ sources once the Rust adapter passes the existing Phase 5 stress test (`test/phase5/stress_adapter.cpp`).
  - **What can go wrong**: SystemC TLM socket has subtleties around back-pressure and transaction ordering that the C++ adapter handles implicitly via TLM semantics. The Rust replacement must explicitly replicate the same ordering guarantees — document this in the crate's module-level doc.
- [ ] **30.9.1** Migrate `test/phase5/stress_adapter.cpp` to Rust.
  - **What**: Rewrite the Phase 5 co-simulation stress test adapter (90 lines) as a Rust binary. It opens a Unix socket, exchanges MMIO request/response packets in a tight loop, and echoes data back — a pure protocol exerciser.
  - **Why**: The stress adapter is the primary correctness and performance gate for `mmio-socket-bridge`. Having it in Rust means it shares `virtmcu-api` types directly (no independent C struct definitions that can drift), and it can be run under `cargo test` as a library unit test without spawning a subprocess.
  - **Depends on**: 30.9 (shares the same protocol types and test infrastructure).
  - **Steps**:
    1. Add a `tools/rust/stress-adapter/` binary crate (or integrate as an integration test in `mmio-socket-bridge`).
    2. Port the socket accept loop and MMIO echo logic using `virtmcu-api` `MmioReq`/`SyscMsg` types.
    3. Update `test/phase5/` pytest to launch the Rust binary instead of the C++ one.
    4. Delete `test/phase5/stress_adapter.cpp` once the Rust version passes the existing stress test suite.
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).

### [Future] Connectivity Expansion
*Depends on: Core simulation loops and bus bridges*
- [ ] **Phase 23**: Bluetooth (nRF52840 RADIO emulation).
- [ ] **Phase 24**: CAN FD (Bosch M_CAN).
- [ ] **Phase 26**: Automotive Ethernet (100BASE-T1).
- [ ] **Phase 28**: Full Digital Twin (Multi-Medium Coordination).

---

## 4. Technical Debt & Future Risks

| ID | Risk | Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Strictly pin QEMU version; track upstream `accel/tcg` changes. |
| R7 | `icount` performance | Only use `slaved-icount` when sub-quantum precision is mandatory. |
| R11 | Zenoh session deadlocks | Implement non-blocking shutdown in `virtmcu-zenoh` helper. |
| R14 | High MTU WiFi/Eth latency | Use lock-free MPSC channels for packet injection. |
| R18 | No firmware coverage measurement | Binary fidelity is the #1 invariant but we have no `drcov`/coverage gate to prove peripherals exercise firmware code paths. Phase 30.8. |
| R19 | `cargo audit` / `cargo deny` soft-fail in `make lint` | Both tools are skipped if not installed (warning only). In CI container they must be hard-required: change skip to exit 1. |
| R20 | `remote-port` payload endianness implicit LE | `bridge_write` now uses `to_le_bytes()` and read-back uses `from_le_bytes()`. The SystemC peer also assumes LE, so this is correct — but if a BE host is ever used, `DEVICE_NATIVE_ENDIAN` must become `DEVICE_LITTLE_ENDIAN`. |

---

## 5. Permanently Rejected / Won't Do
- Python-in-the-loop for clock sync (ADR-001).
- Windows Native Support (QEMU module loading issues).
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
