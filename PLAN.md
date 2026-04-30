# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation. The software MUST be at the highest Enteprise Quality following the SOTA of software development.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

## 1. General Guidelines & Mandates

### Phase Lifecycle
Once a Phase is completed and verified, it MUST be moved from `PLAN.md` to the `/docs/guide/05-project-history.md` file to maintain a clean roadmap and a clear historical record.

### Educational Content (Tutorials)
For every completed phase, a corresponding tutorial lesson MUST be added in `/tutorial`.
- **Target**: CS graduate students and engineers.
- **Style**: Explain terminology, provide reproducible code, and teach practical debugging skills.

### Regression Testing
For every completed phase, an automated integration test MUST be added to `tests/` or `tests/fixtures/guest_apps/`.
- **Bifurcated Testing**:
  - **White-Box (Rust)**: Use `cargo test` for internal state, memory layouts, and protocol parsing.
  - **Black-Box (Python)**: Use `pytest` for multi-process orchestration (QEMU + Zenoh + TimeAuthority).
  - **Thin CI Wrappers (Bash)**: Bash scripts should only be 2-3 lines calling `pytest` or `cargo test`.

### Production Engineering Mandates
- **Environment Agnosticism**: No hardcoded paths. Use `tmp_path` for artifacts.
- **Explicit Constants**: No magic numbers. Use descriptive `const` variables.
- **The Beyonce Rule**: Prove bugs with a failing test before fixing.
- **Lint Gate**: `make lint` must pass before every commit (ruff, version checks, cargo clippy -D warnings).

## 2. Open Items — Ordered by Priority

> **Last updated**: 2026-04-29 (audit of `close_P0s` branch, commit `f45f676`).
> **Mandatory before every commit**: `make lint && make test-unit` must both pass.
> Completed P0 history is in `docs/guide/05-project-history.md`.

### Execution Order (Fundamentals Before Features)

1. [x] **ARCH-20: Eradicate asyncio.sleep in tests** (Status: 🟢 Completed)
2. [x] **DET-6: Wireless Topology & Broadcast Delivery** (Status: 🟢 Completed)
3. [x] **ARCH-8: Hardened Multi-Quantum Barrier** (Status: 🟢 Completed)
4. [x] **DET-3: Hardware Jitter Profile Injection (Chaos Engineering)** (Status: 🟢 Completed)

---

### **[DET-3] Hardware Jitter Profile Injection (Chaos Engineering)**

**Goal**: Extend the `SimulationTransport` to include a `FaultInjectingTransport` wrapper to simulate packet loss, delay, and network jitter.
**What needs to be improved**: Deterministic tests currently prove functionality under ideal conditions. Enterprise quality demands proving the system survives terrible network conditions without deadlocking the coordinator or dropping execution constraints.
**How it's tested**: Create a parameterized test matrix (`pytest.mark.parametrize("faults", ["none", "drop_5_percent", "delay_10ms"])`). Assert that the firmware and `deterministic_coordinator` handle retries and timeouts gracefully and simulation time remains accurately bounded.

---

### **[Recently Completed P0 Audit & Hardening]**

| Task | Scope | Status |
|---|---|---|
| A — P02 | `remote-port`: replace unaligned cast reads with `ptr::read_unaligned` | ✅ CLOSED |
| B — P05 | Consolidate dual locking to pure Rust `Mutex`/`Condvar` in both bridges | ✅ CLOSED |
| C — P06 | Fix teardown UAF: `VcpuCountGuard` RAII + `drain_cond.wait_timeout(30s)` | ✅ CLOSED |
| D — P07 | Replace `thread::sleep` reconnect loop with `Condvar.wait_timeout` | ✅ CLOSED |
| E — P-SERIAL | Eliminate `transmute` and raw memory views; explicit `pack()`/`unpack()` | ✅ CLOSED |
| F — P-SCHEMA | Migrate `vproto.py` to FlatBuffers (core.fbs) | ✅ CLOSED |
| G1 | Replace `slice::from_raw_parts` on RP packet sends with `pack_be()` | ✅ CLOSED |
| G2 | `VcpuCountGuard` RAII added to both bridges to fix panic-safety | ✅ CLOSED |
| H | `BqlGuarded<T>` migration for all Zenoh peripherals + Mutex lint | ✅ CLOSED |
| I | Fix Docker Bake tag replacement behavior (prevent manifest failures) | ✅ CLOSED |
| J | Definitive fix for ARCH-8 race condition (Unified Lock + Lookahead) | ✅ CLOSED |

**Audit findings fixed on top of G (2026-04-29)**:
- `bridge_write` in `remote-port` used `to_ne_bytes()` (implicit LE-host assumption) → fixed to `to_le_bytes()`.
- Read-back in `send_req_and_wait_internal` used raw `ptr::copy_nonoverlapping` into `&mut u64` → fixed to `u64::from_le_bytes()`.
- `zenoh-spi` used raw `ptr::copy_nonoverlapping` for header serialization → fixed to `ZenohSPIHeader::pack()`.
- BqlGuarded<T> introduced in virtmcu-qom to eliminate redundant Mutex usage in BQL-held contexts.
- Mutex<T> banned in zenoh-* peripherals via make lint gate (except validated background threads).
- Byte-exact pack_be() tests added for RpPktBusaccess and RpPktInterrupt.
- Leftover Gemini one-shot patch scripts deleted from repo root.
- Fixed Zenoh connection hangs by removing non-deterministic liveliness condvar waits from transport-zenoh.
- Fixed QEMU plugin dynamic loading by enforcing visibility("default") on globally injected virtmcu hook setters.
- Resolved double-instantiation bugs for CLI-only peripherals (telemetry, ieee802154) in yaml2qemu.py.
- Fixed test_telemetry_stress_queue timeout by removing a race condition with the QMP RESUME event listener.
- Executed SOTA architectural review on QOM dynamic plugin loading. Confirmed that single-instantiation of sysbus devices via DTB injection (bypassing redundant -device flags) provides the cleanest, most robust memory-mapping solution for peripherals like ieee802154.
- **Hardened ARCH-8 Barrier**: Identified and fixed a non-deterministic race where fast nodes could finish quantum N+1 before the coordinator processed N. Implemented unified state locking and a 1-quantum lookahead buffer in `QuantumBarrier`.
- **DTC Hardening**: Updated FDT emitter to treat DTC warnings as non-fatal, preventing CI failures for valid fragmented trees.

---

### **[ARCH-20] Eradicate asyncio.sleep in tests** (Status: 🟢 Completed)

**Goal**: Replace all non-deterministic wall-clock `asyncio.sleep()` calls in `tests/` with deterministic `vta.step()`, QMP events, or Zenoh `recv_async()`/`liveliness()` checks.

**Definition of Done**:
- [x] No unwarranted `asyncio.sleep` calls remain.
- [x] `make lint` fails if new unannotated `asyncio.sleep` calls are added.
- [x] Tests remain stable without wall-clock sleeps.

---

**Determinism migration (new — highest correctness priority):**
1. **DET-9** — Wireshark extcap plugin (lowest priority).

**Hardware / infrastructure (existing, continue in parallel with DET work):**
2. **Phase 27** — FlexRay IRQs + Bosch E-Ray Message RAM.
3. **Phase 21 / 22** — WiFi / Thread Protocol expansion.
4. **Phase 30.9 + 30.9.1** — Rust systemc-adapter + stress-adapter.
5. **Phase 30.8 + 30.10** — Firmware coverage (drcov) + unified reporting.
6. **P12** — Deterministic Deadlock Detection (virtual-time budgets).
7. **Phase 31** — Vendor Firmware Validation (Ethernet & CAN-FD Binary Fidelity).

---

### Completed P0 Serial Work (Tasks A–G) — Summary

| Task | Scope | Status |
|---|---|---|
| A — P02 | `remote-port`: replace unaligned cast reads with `ptr::read_unaligned` | ✅ CLOSED |
| B — P05 | Consolidate dual locking to pure Rust `Mutex`/`Condvar` in both bridges | ✅ CLOSED |
| C — P06 | Fix teardown UAF: `VcpuCountGuard` RAII + `drain_cond.wait_timeout(30s)` | ✅ CLOSED |
| D — P07 | Replace `thread::sleep` reconnect loop with `Condvar.wait_timeout` | ✅ CLOSED |
| E — P-SERIAL | Eliminate `transmute` and raw memory views; explicit `pack()`/`unpack()` | ✅ CLOSED |
| F — P-SCHEMA | Migrate `vproto.py` to FlatBuffers (core.fbs) | ✅ CLOSED |
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

**Required deliverable**: Migration of `tools/vproto.py` to FlatBuffers.

**Outcome**: Successfully migrated the core protocol IDL to **FlatBuffers** (`core.fbs`). `tools/vproto.py` is now a manual wrapper around the `flatc`-generated Python bindings, providing `@dataclass` ergonomics with FlatBuffers performance and schema safety. The previous plan for a `gen_vproto.py` script was deprecated as FlatBuffers provides the necessary cross-language type safety.

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

### [Hardware] Phase 24 — CAN-FD (Bosch M_CAN) 🚧
*Depends on: Phase 19 (Rust QOM) ✅*
- [ ] **24.1** Implement missing Bosch M_CAN register logic.
- [ ] **24.2** Enable and verify CAN-FD frame payload delivery over Zenoh.
- [ ] **24.3** Pass Vendor SDK loopback/echo tests (Link to Phase 31.1).

### [Hardware] Phase 27 — FlexRay (Automotive) 🚧
*Depends on: Phase 5 (Bridge) ✅, Phase 19 (Rust QOM) ✅*
- [ ] **27.1.1** Add FlexRay Interrupts (IRQ lines).
- [ ] **27.1.2** Implement Bosch E-Ray Message RAM Partitioning.
- [ ] **27.2.1** Fix SystemC build regression (CMake 4.3.1 compatibility).

### [Hardware] Phase 21 — WiFi (802.11) 🚧
*Depends on: Phase 20.5 (SPI)*
- [ ] **21.7.1** Harden `arm-generic-fdt` Bus Assignment (Child node auto-discovery).
- [ ] **21.7.2** Formalize `wifi` Rust QOM Proxy.
- [ ] **21.2** Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### [Hardware] Phase 22 — Thread Protocol 🚧
*Depends on: Phase 20.5 (SPI), Phase 21 (WiFi)*
- [ ] **22.1** Deterministic Multi-Node UART Bus Bridge.
- [ ] **22.2** SPI 802.15.4 Co-Processor (e.g., AT86RF233).

### [Infrastructure] Phase 30 — Deep Oxidization & Testing Overhaul 🚧
*Ongoing*
- [ ] **30.8** Comprehensive Firmware Coverage (drcov integration).
- [ ] **30.9** Migrate `tools/systemc_adapter/` to Rust (`tools/rust/systemc-adapter/`).
  - Rewrite `main.cpp` (662 lines) + `remote_port_adapter.cpp` (96 lines) as a Rust binary sharing `virtmcu-api` types directly.
  - Add smoke test: Rust adapter ↔ `mmio-socket-bridge` round-trip MMIO read.
  - Deprecate C++ once Rust adapter passes Phase 5 stress test.
- [ ] **30.9.1** Migrate `tests/fixtures/guest_apps/phase5/stress_adapter.cpp` to Rust (depends on 30.9).
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).

### [Infrastructure] INFRA-7 — Automated Flight Recorder (Record & Replay)
**Status**: 🟡 Open.
**Goal**: Record all simulation traffic to a PCAP or JSON artifact upon test failure for immediate offline debugging.
**What needs to be improved**: Debugging CI failures currently requires reading thousands of lines of verbose logs. Reproducing a CI-only failure locally requires full recompilation and manual log alignment.
**How it's tested**: Inject a recording hook into `SimulationTransport`. On `pytest` teardown, if the test failed, save the buffer to an artifact. Test by intentionally asserting a failure in a mock test and validating the generated `.pcap` matches the exact UART/Zenoh message sequence.

### [Hardware] Phase 31 — Vendor Firmware Validation (Binary Fidelity) 🚧
**Status**: 🟡 Open.

**Goal**: To guarantee true binary fidelity, VirtMCU must be validated against official, unmodified vendor SDK binaries targeting specific, named hardware peripherals. "Generic" bare-metal tests are insufficient for complex IP blocks.

**Mandates for Reference Materials**:
1. **Zero-Commit Policy for Imported Code**: Official vendor SDK examples, libraries, or firmware source code MUST NOT be committed to the repository. Store them in `third_party/golden_references/<mcu_name>/` (which is tracked via `.gitkeep` but contents are ignored).
2. **Datasheet & Spec PDFs**: Official peripheral datasheets and board spec PDFs MUST be stored in the same `third_party/golden_references/<mcu_name>/` folder. These files reside in the local filesystem for developer reference but MUST NOT be checked into revision control.
3. **Reference READMEs (Tracked)**: For every new real peripheral reference (SDK, code, or spec PDF) added to `third_party/golden_references/`, a `README.md` MUST be created in its respective MCU subfolder. This `README.md` MUST be committed to version control and contain: 
   - The original download URL / source.
   - The license under which it is distributed.
   - The exact date of download.
4. **Reproducible Provenance**: Every firmware in `tests/firmware/` must have a corresponding `PROVENANCE.md` providing a direct download link and clear instructions for re-acquiring the original vendor materials stored in `third_party/golden_references/`.

**Tasks**:
- [ ] **31.1** **CAN-FD (Bosch M_CAN)**: 
...
  - *Target*: Identify a specific vendor MCU with a Bosch M_CAN controller (e.g., STM32G4, NXP S32K3).
  - *Action*: Download the official vendor SDK CAN-FD example (e.g., echo/loopback). Compile unmodified and implement the missing M_CAN register logic in VirtMCU (Phase 24) to make the vendor firmware pass.
- [ ] **31.2** **Ethernet (MAC)**:
  - *Target*: Identify a specific vendor MCU/Board with an Ethernet MAC supported by QEMU (e.g., SMSC LAN9118 on Cortex-A15, or NXP ENET on i.MX).
  - *Action*: Download the official vendor SDK lwIP/ping example. Compile unmodified and test against `virtmcu-netdev` to verify bidirectional packet flow.
- [ ] **31.3** **Provenance Enforcement**: Update `tests/firmware/*/PROVENANCE.md` (and create for all new firmwares) to mandate that *all* test firmwares explicitly list the exact real-world MCU, the specific peripheral name (e.g., "NXP S32K144 LPUART0"), the vendor SDK version, and a reproducible download/build link.


### [Infrastructure] INFRA-9 — Execution Pacing & Faster-Than-Real-Time (FTRT) Support
**Status**: 🟡 Open.
**Goal**: Formalize the separation between **Wall-Clock Timeouts** (fail-fast boundaries) and **Simulation Pacing** (controlling guest execution speed relative to reality). VirtMCU must run FTRT in CI, but support interactive real-time (1.0x) or slow-motion (e.g., 10.0x) for human-in-the-loop UI and GDB sessions.
**What needs to be improved**: Tests and runtime environments currently assume "as fast as possible." When connecting a frontend UI or Wireshark, the simulation runs too fast for human observation. Conversely, we must mathematically prove that CI runs FTRT without artificial framework bottlenecks.
**How it's tested**: 
1. **Host Timeout Scale**: Implement `HOST_TIMEOUT_MULTIPLIER` in `conftest_core.py` to transparently stretch/shrink wait boundaries based on ASan/CI runners.
2. **Coordinator Pacing**: Add `--pacing <float>` to `deterministic_coordinator`. `0.0` = FTRT (default), `1.0` = Real-time, `10.0` = Slow motion.
3. **Runtime UI Control**: Expose a QMP/Zenoh endpoint allowing a connected Frontend UI to dynamically adjust the pacing multiplier at runtime.
4. **FTRT Proof Test**: Create a CI test that simulates 60 seconds of virtual stress-load, asserting that it completes in `< 5 seconds` of Wall-Clock time.

### [Future] Connectivity Expansion
- [ ] **Phase 23**: Bluetooth (nRF52840 RADIO emulation).
- [ ] **Phase 26**: Automotive Ethernet (100BASE-T1).
- [ ] **Phase 28**: Full Digital Twin (Multi-Medium Coordination).
- [ ] **DET-9**: Wireshark extcap plugin (Reads the coordinator PCAP log and displays each inter-node message).

## 3. Architectural Hardening — Concurrency, Correctness & Scale

> **Purpose**: Close known concurrency bugs, wire-protocol gaps, and design debt identified
> in the April 2026 deep-architecture review. Tasks are ordered by severity. Each is
> self-contained with exact file paths, step-by-step implementation, tests, and a binary
> definition of done.
>
> **Audience**: AI coding agents and junior engineers. Follow steps exactly. Do not infer.
>
> **Prerequisite**: `make lint && make test-unit` MUST pass before starting any task.

---

### **[ARCH-14] Document and Measure Simulation Frequency Ceiling** — Observability

**Status**: 🟡 Open. Depends on: DET-4 (Unix socket transport).

**Goal**: Document the maximum sustainable quantum rate for each transport option.
Add a benchmark script. Add the measured results as a table in ARCHITECTURE.md so
engineers can choose the right transport for their scenario.

**Files to create**:
- `tools/benchmarks/clock_rtt_bench.py` — measures median clock RTT across 10 000 quanta

**Files to modify**:
- `docs/architecture/01-system-overview.md` — add "Simulation Frequency Ceiling" table

**Definition of Done**:
- [ ] Benchmark script exists and is runnable in CI.
- [ ] Results table added to ARCHITECTURE.md §9.
- [ ] `make lint` (ruff) passes on the benchmark script.

---

### **[ARCH-15] SMP Firmware Quantum Barrier** — Correctness for Dual-Core Firmware

**Status**: 🟡 Open. No dependencies. Low priority unless dual-core firmware is needed.

**Goal**: When QEMU is started with SMP (`-smp 2`), the TCG quantum hook fires
independently on each vCPU thread. Both vCPUs must halt at the quantum boundary before any
`ClockReadyResp` is sent. Implement a per-quantum vCPU barrier counter.

**Files to modify**:
- `hw/rust/backbone/clock/src/lib.rs` — add `n_vcpus: u32` QOM property (default 1);
  add `vcpu_halt_count: AtomicU32`; in the quantum hook, increment the counter and wait
  (using `Condvar`) until `vcpu_halt_count == n_vcpus` before sending `ClockReadyResp`;
  reset counter at quantum start.

**Definition of Done**:
- [ ] `n-vcpus` property added to `clock` device.
- [ ] With `n-vcpus=2` and `-smp 2`, both vCPUs halt before reply is sent.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

### **[ARCH-17] Replace `GLOBAL_CLOCK` Singleton to Support Multi-MCU QEMU** — Architecture

**Status**: 🟡 Open. Low priority. Depends on: ARCH-1 and ARCH-3 complete.

**Goal**: Replace process-wide `GLOBAL_CLOCK` with a per-device-instance registry keyed by
node ID, allowing multiple independent clock devices per QEMU process.

**Files to modify**:
- `hw/rust/backbone/clock/src/lib.rs` — replace `static GLOBAL_CLOCK` with `static CLOCK_REGISTRY: Mutex<HashMap<u32, Arc<ZenohClock>>>`.

**Definition of Done**:
- [ ] `GLOBAL_CLOCK: AtomicPtr` removed.
- [ ] `CLOCK_REGISTRY: Mutex<HashMap<u32, Arc<ZenohClock>>>` introduced.
- [ ] `test_two_clock_instances_independent` passes.
- [ ] `make lint` passes.

---

### **[ARCH-19] Transport-Agnostic Data Plane & Restructure** — Architecture & Testing (Status: ✅ Completed)

**Status**: ✅ Completed.

**Goal**: Abstract the data plane transport so that peripherals do not hardcode Zenoh pub/sub, and organize `hw/rust/` into logical subdirectories. Organizes `hw/rust/` into `backbone/`, `comms/`, `observability/`, `mcu/`, and `common/`.

**Definition of Done**:
- [x] `DataTransport` trait established in `virtmcu-api`.
- [x] Crates renamed to remove `zenoh-` prefix where appropriate.
- [x] `topology.transport` parsed and applied during initialization.
- [x] `deterministic_coordinator` supports both Zenoh and Unix socket listeners.
- [x] Peripherals refactored to use the abstract transport.
- [x] All paths updated across all project documents to reflect the new structure.
- [x] `make lint` passes.

## 5. Ongoing Risks (Watch List)

Items here have no immediate action — they are structural constraints or future triggers to monitor.

| ID | Risk | Status / Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Ongoing. QEMU version is pinned; all patches go through `scripts/apply-qemu-patches.sh`. Track upstream `accel/tcg` changes on each QEMU bump. |
| R7 | `icount` performance | Design guideline: use `slaved-icount` only when sub-quantum timing precision is required. `slaved-suspend` is the default. |
| R11 | Zenoh session deadlocks in teardown | Partially mitigated: `SafeSubscriber` calls `undeclare().wait()` in `drop()`. |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov`/coverage CI gate. Tracked as Phase 30.8. |

## 6. Permanently Rejected / Won't Do
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
- [x] Fixed Miri tests across the workspace

## Completed Operations (FlatBuffers Migration & Stabilization)
- Migrated core IDL (networking & mmio headers) from manual packed C structs to rigorous FlatBuffers definitions (`core.fbs`).
- Surgically updated all Python parsing endpoints dynamically using `vproto.py` to prevent size-boundary mismatches.
- Authored `docs/guide/03-testing-strategy.md` and `docs/architecture/04-communication-protocols.md`.

### Architectural Critique: ARCH-20 Follow-up

- **Race Conditions in Polling**: Fixed by ensuring dedicated reader events/conditions in `QmpBridge` and `AsyncManagedProcess`.
- **Suboptimal CPU usage (Pseudo-polling)**: Fully mitigated using actual `asyncio.Event` constructs inside subscriber callbacks. 
- **Exception for Virtual Time Polling**: `QmpBridge` still executes `asyncio.wait(..., timeout=0.1)` when querying QMP for virtual time (documented constraint).
- **Exception for Rate Limiting**: Retained legitimately for traffic shaping in stress tests.

All constraints and corner-cases have been validated under ASan load scaling.
