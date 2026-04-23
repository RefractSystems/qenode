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

> Tasks are sequenced. A task marked **[UNBLOCKED]** can start immediately. A task marked **[NEEDS Pxx]** must wait for that task to finish. Complete each task with a passing `make lint` + stress test before moving to the next.

---

### **[P01 — UNBLOCKED] Fix Main Branch Test Failures**
**Goal**: Restore the `main` branch to a fully green state before any refactoring.
**Why first**: All subsequent P0 work will generate commits on `main`. A green baseline is required to detect regressions introduced by each change.
**Definition of Done**: `make ci-full` exits 0 inside the devcontainer on a clean clone, and the GitHub Actions pipeline shows all jobs green for two consecutive runs.
1. Run `gh run list --limit 5` and `gh run view --log` to enumerate every failing test by name.
2. Run each failing test in isolation (`pytest tests/test_foo.py -v`) and then under load (`pytest tests/test_foo.py -n auto --count=20`) to distinguish flakes from deterministic failures.
3. Apply surgical fixes to restore stability.
4. Once individually passing, run the full suite 3 times end-to-end. Target: zero failures across all runs.

---

### **[P02 — UNBLOCKED] Fix Unaligned Packed Struct Read UB in `remote-port`**
**Goal**: Eliminate undefined behavior. This is a correctness bug independent of all locking work.
- **Location**: `remote-port/src/lib.rs:307` — `let hdr_be = unsafe { *(rx_buf.as_ptr() as *const RpPktHdr) }`. `rx_buf` is a `Vec<u8>` (align=1). Reading `#[repr(C, packed)]` via cast pointer is UB when the compiler generates aligned loads for fields during destructuring.
- **Fix**: Replace all packed struct reads from byte buffers with `ptr::read_unaligned`. Apply to `RpPktHdr`, `RpPktBusaccess`, and `RpPktInterrupt`.
- **Verification**: Add `#[cfg(test)]` unit tests using a byte slice offset by 1 to prove the reads are correct under misalignment.

---

### **[P03 — UNBLOCKED] Upgrade `sync.rs` Mock & Add Unit Tests**
**Goal**: The BQL abstraction layer must have tested invariants before any peripheral refactoring relies on it.
- **Problem**: The test mock in `sync.rs` returns `1` (signaled) unconditionally from `virtmcu_cond_timedwait`, making timeout paths permanently invisible to unit tests.
- **Steps**:
  1. Upgrade the mock to use a `thread_local` or per-test state that supports configurable return values (signaled vs. timed-out).
  2. Write `#[test]` unit tests covering: `BqlGuard` drop releases lock; `BqlUnlockGuard` re-acquires on drop; `temporary_unlock()` returns `None` when BQL is not held; `wait_timeout` timeout path (mock returns 0); `wait_timeout` signal path (mock returns 1).
- **Why before P04**: P04 depends on `wait_yielding_bql` being tested and correct before it is used in peripherals.

---

### **[P04 — NEEDS P03] Enterprise BQL Safety: Implement `wait_yielding_bql` & Ban Raw FFI**
**Goal**: Provide the one approved way to block a vCPU thread on a CondVar. Eliminate all direct FFI calls to `virtmcu_mutex_lock/unlock` from peripheral code.
- **What**: Implement `QemuCond::wait_yielding_bql<'a>(&mut self, guard: QemuMutexGuard<'a>, timeout_ms: u32) -> (QemuMutexGuard<'a>, bool)` in `virtmcu-qom/src/sync.rs`. **Full contract**: caller passes ownership of a `QemuMutexGuard` (proving the peripheral mutex is locked). The function: (1) releases the BQL via `Bql::temporary_unlock()`; (2) calls `wait_timeout` on the CondVar; (3) re-acquires BQL (BqlUnlockGuard drop); (4) returns the guard and a bool (true = signaled, false = timed out). The peripheral mutex is held on both entry and exit.
- **Also implement**: `QemuCond::wait_yielding_bql_loop` — same contract but loops until a predicate returns true or a timeout expires. This eliminates the while-loop boilerplate in every bridge.
- **Audit**: After implementing, grep the entire `hw/rust/` tree for direct calls to `virtmcu_mutex_lock`, `virtmcu_mutex_unlock`, `virtmcu_bql_lock`, `virtmcu_bql_unlock` outside of `virtmcu-qom/src/sync.rs`. Each must be eliminated.
- **BANNED after this task**: Direct FFI calls to BQL or peripheral mutex primitives from any peripheral crate.
- **What can go wrong**: AB-BA deadlocks during refactoring. The new abstraction must guarantee BQL is re-acquired in all code paths including panics — use `scopeguard` or a drop impl.

---

### **[P05 — NEEDS P04] Dual Locking Scheme Consolidation**
**Goal**: Each peripheral uses exactly one locking scheme. The current mix of Rust `std::sync::Mutex` + raw QEMU `*mut QemuMutex` per device creates undocumented lock ordering and makes `wait_yielding_bql` impossible to call correctly.
- **Decision** (applies to `mmio-socket-bridge` and `remote-port`):
  - **Adopt Option A**: Use Rust `std::sync::Mutex` + `std::sync::Condvar` for all state. Manage BQL with `Bql::temporary_unlock()`. Remove the raw `*mut QemuMutex` / `*mut QemuCond` entirely.
  - Rationale: Rust mutexes are panic-safe (poisoning), the borrow checker enforces guard lifetimes, and they don't require heap allocation via `virtmcu_mutex_new()`.
- **Document**: Add a module-level doc comment to every peripheral file stating the lock order: `BQL → peripheral Mutex → (Condvar releases Mutex temporarily)`.

---

### **[P06 — NEEDS P05] Fix Device Teardown UAF — Shutdown Safety**
**Goal**: Eliminate the latent Use-After-Free in bridge device teardown.
- **Root cause**: `bridge_instance_finalize` in `mmio-socket-bridge` uses a bounded spinloop (`attempts < 1000` × `yield_now()`) as a drain. If exhausted while a vCPU thread is blocked in `send_req_and_wait_internal`, the mutex/condvar is freed while in use — a UAF. `remote-port` has no drain at all.
- **Steps**:
  1. (After P05, using Rust Condvar): Add an `active_vcpu_count: AtomicUsize` and a `drain_cond: Arc<Condvar>` to `SharedState`.
  2. Decrement `active_vcpu_count` on exit from `send_req_and_wait`. When it reaches 0, call `drain_cond.notify_all()`.
  3. In `bridge_instance_finalize`: set `running=false` → signal `resp_cond` (wakes blocked vCPU threads) → `drain_cond.wait_timeout()` until `active_vcpu_count == 0` → join bg_thread → drop state.
  4. Apply the same pattern to `remote-port`.
- **Verification**: Shutdown stress test (pytest or Rust integration test) that boots QEMU with a connected bridge, issues a blocking MMIO read from firmware, then sends QMP `quit`, and asserts clean exit under ASan.
- **What can go wrong**: Ordering — `resp_cond.notify_all()` must fire *before* `drain_cond.wait()`, otherwise the vCPU thread never wakes up to decrement the count.

---

### **[P07 — NEEDS P05] Eradication of `std::thread::sleep` in `hw/rust/`**
**Goal**: Remove all wall-clock sleeps in peripheral code. After P05 the Condvar infrastructure exists to replace them.
- **Current violations** (must all be eliminated):
  - `mmio-socket-bridge/src/lib.rs:116` — reconnect retry sleep
  - `mmio-socket-bridge/src/lib.rs:227` — connection-wait sleep before send
  - `remote-port/src/lib.rs:244, 334, 454` — same patterns
  - `zenoh-clock/src/lib.rs:710` — heartbeat thread sleep
- **Fix for bridges**: Add a `connected_cond: Condvar` to `SharedState`. Background thread notifies on connect. vCPU thread waits on it (with BQL released) instead of sleeping.
- **Fix for zenoh-clock heartbeat**: Replace `thread::sleep(1s)` with `backend.cond.wait_timeout(guard, 1s)` — wakes immediately when `shutdown` is set.
- **CI enforcement**: Add to `make lint-rust`: `grep -rn "thread::sleep" hw/rust/ --include="*.rs" | grep -v "//.*SLEEP_EXCEPTION"` must find zero matches. Fail build if any found.

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
| R15 | `sync.rs` mock gives false test confidence | Mock's `virtmcu_cond_timedwait` always returns 1 (signaled); timeout paths are untestable. Upgrade mock to simulate lock state and support configurable return values. |
| R16 | Bridge teardown UAF under shutdown race | `active_vcpu_count` spinloop in `mmio-socket-bridge` and missing drain in `remote-port`. Fix with condvar drain (see P0 task). |
| R17 | Unaligned packed struct reads in `remote-port` | `RpPktHdr`, `RpPktBusaccess`, `RpPktInterrupt` read via cast pointer = UB. Replace with `ptr::read_unaligned`. |
| R18 | No firmware coverage measurement | Binary fidelity is the #1 invariant but we have no `drcov`/coverage gate to prove peripherals exercise firmware code paths. Phase 30.8. |
| R19 | `cargo audit` / `cargo deny` soft-fail in `make lint` | Both tools are skipped if not installed (warning only). In CI container they must be hard-required: change skip to exit 1. |

---

## 5. Permanently Rejected / Won't Do
- Python-in-the-loop for clock sync (ADR-001).
- Windows Native Support (QEMU module loading issues).
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
