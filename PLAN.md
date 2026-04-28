# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation. The software MUST be at the highest Enteprise Quality following the SOTA of software development.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

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

## 2. Open Items — Ordered by Priority

> **Last updated**: 2026-04-27 (audit of `close_P0s` branch, commit `74f13df`).
> **Mandatory before every commit**: `make lint && make test-unit` must both pass.
> Completed P0 history is in `docs/COMPLETED_PHASES.md`.

### Execution Order (Fundamentals Before Features)

1. [x] **ARCH-20: Eradicate asyncio.sleep in tests** (Status: 🟢 Completed)
2. [ ] **DET-3: Hardware Jitter Profile Injection (Chaos Engineering)** (Status: 🟡 Open)

---

### **[DET-3] Hardware Jitter Profile Injection (Chaos Engineering)**

**Goal**: Extend the `SimulationTransport` to include a `FaultInjectingTransport` wrapper to simulate packet loss, delay, and network jitter.
**What needs to be improved**: Deterministic tests currently prove functionality under ideal conditions. Enterprise quality demands proving the system survives terrible network conditions without deadlocking the coordinator or dropping execution constraints.
**How it's tested**: Create a parameterized test matrix (`pytest.mark.parametrize("faults", ["none", "drop_5_percent", "delay_10ms"])`). Assert that the firmware and `deterministic_coordinator` handle retries and timeouts gracefully and simulation time remains accurately bounded.

---

### **[ARCH-20] Eradicate asyncio.sleep in tests** (Status: 🟢 Completed)

**Goal**: Replace all non-deterministic wall-clock `asyncio.sleep()` calls in `tests/` with deterministic `vta.step()`, QMP events, or Zenoh `recv_async()`/`liveliness()` checks.

**Requirements**:
1. Scan the `tests/` directory for `asyncio.sleep`.
2. For each usage, determine if it is waiting for a QEMU boot state (should use QMP), a network event (should use Zenoh subscription/liveliness), or virtual time advancement (should use `vta.step()`).
3. Replace the `asyncio.sleep()` with the deterministic construct.
4. Add a `grep -r "asyncio.sleep" tests/` check to the `lint-python` target in `Makefile` to ban it in the future, supporting `# SLEEP_EXCEPTION: <reason>` for the very few places (like testing the stall-timeout mechanism) where it is truly required.

**Definition of Done**:
- [ ] No unwarranted `asyncio.sleep` calls remain.
- [ ] `make lint` fails if new unannotated `asyncio.sleep` calls are added.
- [ ] Tests remain stable without wall-clock sleeps.

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
- [ ] **30.9.1** Migrate `test/phase5/stress_adapter.cpp` to Rust (depends on 30.9).
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).

### [Infrastructure] INFRA-6 — Centralized Timeout Multiplier & ASan Scaling
**Status**: 🟡 Open.
**Goal**: Decouple logical timeouts from execution environments (ASan, TSan, CI) to prevent flaky tests without resorting to arbitrary massive hardcoded sleeps.
**What needs to be improved**: Tests currently rely on manual scaling or arbitrarily large timeouts to survive ASan/TSan overhead. This hides actual deadlocks and makes tests unreadable.
**How it's tested**: Implement a `get_time_multiplier()` function in `conftest_core.py`. Verify that tests using `timeout=2.0` automatically wait `10.0` seconds under ASan. Assert that QEMU's `stall-timeout` parameter is dynamically multiplied via `qemu_launcher` before QEMU instantiation.

### [Infrastructure] INFRA-7 — Automated Flight Recorder (Record & Replay)
**Status**: 🟡 Open.
**Goal**: Record all simulation traffic to a PCAP or JSON artifact upon test failure for immediate offline debugging.
**What needs to be improved**: Debugging CI failures currently requires reading thousands of lines of verbose logs. Reproducing a CI-only failure locally requires full recompilation and manual log alignment.
**How it's tested**: Inject a recording hook into `SimulationTransport`. On `pytest` teardown, if the test failed, save the buffer to an artifact. Test by intentionally asserting a failure in a mock test and validating the generated `.pcap` matches the exact UART/Zenoh message sequence.

### [Infrastructure] INFRA-8 — Host vs. Guest Hang Detection & Environment Markers
**Status**: 🟡 Open.
**Goal**: Explicitly detect when QEMU deadlocks vs when the Guest RTOS faults, and cleanly gate unsupported environments.
**What needs to be improved**: Global `pytest-timeout` currently triggers on all hangs, leaving total ambiguity about the root cause (Host vs Guest). `ASan` constraints are currently handled arbitrarily via `if` statements.
**How it's tested**: Implement an out-of-band watchdog in Python that queries `get_virtual_time_ns()`. If virtual time stalls for 5+ iterations while wall-clock advances by 10s, immediately fail with a "Guest OS deadlocked" exception. Add `@pytest.mark.skip_asan` markers and verify `pytest -m "not skip_asan"` properly excludes timing-sensitive tests from ASan pipelines.

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
- `hw/rust/backbone/clock/src/lib.rs` — add `n_vcpus: u32` QOM property (default 1);
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
- [ ] `n-vcpus` property added to `clock` device.
- [ ] With `n-vcpus=2` and `-smp 2`, both vCPUs halt before reply is sent.
- [ ] Both unit tests pass.
- [ ] `make lint` passes.

---

### **[ARCH-17] Replace `GLOBAL_CLOCK` Singleton to Support Multi-MCU QEMU** — Architecture

**Status**: 🟡 Open. Low priority. Depends on: ARCH-1 and ARCH-3 complete.

**Goal**: `GLOBAL_CLOCK` is a process-wide singleton. If a user instantiates two
`clock` devices in one QEMU process (e.g., a dual-MCU board), the second
instantiation overwrites the first. Replace with a per-device-instance registry keyed by
node ID, allowing multiple independent clock devices per QEMU process.

**Files to modify**:
- `hw/rust/backbone/clock/src/lib.rs` — replace `static GLOBAL_CLOCK: AtomicPtr<ZenohClock>`
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

### **[ARCH-19] Transport-Agnostic Data Plane & Restructure** — Architecture & Testing (Status: ✅ Completed)

**Status**: ✅ Completed.

**Goal**: Abstract the data plane transport so that peripherals do not hardcode Zenoh pub/sub, and organize `hw/rust/` into logical subdirectories (dropping the `zenoh-` prefix). The transport layer (Zenoh vs. Unix Domain Sockets) should be dynamically chosen based on the world YAML `topology.transport` setting. 

**Requirements**:
1. [x] **Directory Restructure**: Reorganize `hw/rust/` and drop prefixes:
   - `backbone/`: `clock`, `mmio-socket-bridge`, `remote-port`, `transport-zenoh`, `transport-unix`.
   - `comms/`: `netdev`, `chardev`, `canfd`, `flexray`, `spi`, `ieee802154`, `wifi`.
   - `observability/`: `actuator`, `telemetry`, `ui`.
   - `mcu/`: `s32k144-lpuart`.
   - `common/`: `virtmcu-api`, `virtmcu-qom`.
2. [x] **Abstract Data Plane**: Introduce a `DataTransport` trait (or similar) in `virtmcu-api` to abstract away Zenoh pub/sub.
3. [x] **YAML Configuration**: Add a `transport` key to the `topology:` section in `worlds/*.yaml` (e.g., `transport: unix` or `transport: zenoh`).
4. [x] **Multi-Protocol Coordinator**: Update `deterministic_coordinator` to listen on either Unix Domain Sockets or Zenoh depending on the configuration.
5. [x] **Universal Test Coverage**: Ensure integration tests run both Unix Socket and Zenoh transports. All peripheral implementations must use the abstract API.
- [x] Documentation: Add a `README.md` to `hw/rust/` documenting this new layout.

7. [x] **Path Updates**: Update all paths across all documents (e.g., `PLAN.md`, `CLAUDE.md`, etc.) to reflect the new directory structure before closing this task.
**Status**: ✅ Completed.
...
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
| R7 | `icount` performance | Design guideline: use `slaved-icount` only when sub-quantum timing precision is required (PWM, µs-level). `slaved-suspend` is the default. |
| R11 | Zenoh session deadlocks in teardown | Partially mitigated: `SafeSubscriber` calls `undeclare().wait()` in `drop()`. Remaining risk: calling `.wait()` from inside a Zenoh callback context can still deadlock. Monitor for new peripherals that add Zenoh callbacks with complex teardown. |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov`/coverage CI gate to prove peripherals exercise firmware code paths. Tracked as Phase 30.8. |

## 6. Permanently Rejected / Won't Do
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
- [x] Fixed Miri tests across the workspace

## Completed Operations (FlatBuffers Migration & Stabilization)
- Migrated core IDL (networking & mmio headers) from manual packed C structs to rigorous FlatBuffers definitions (`core.fbs`).
- Surgically updated all Python parsing endpoints dynamically using `vproto.py` to prevent size-boundary mismatches (eliminating 126 manual `struct.pack` usages and hardcoded `[24:]` slices).
- Authored `docs/TEST_GUIDELINES.md` and `docs/DATA_FLOW_AND_PROTOCOLS.md` standardizing testing templates, IDL data flows, and Python interaction mandates for new developers.

### Architectural Critique: ARCH-20 Follow-up

- **Race Conditions in Polling**: `QmpBridge` and `AsyncManagedProcess` previously used `.clear()` on `asyncio.Event()` objects, which created a race condition if multiple coroutines attempted to `wait_for_line` or `wait_for_event` concurrently. This has been solved or carefully segregated by ensuring dedicated reader events/conditions.
- **Suboptimal CPU usage (Pseudo-polling)**: Many scripts were simply spinning with `asyncio.sleep(0.01)` to wait for Zenoh messages or backend processes. This defeats the purpose of the deterministic eradication effort. They have now been fully mitigated using actual `asyncio.Event` constructs inside the subscriber callbacks (e.g. in `test_phase12.py`, `test_chardev_bql_stress.py`, `test_det5_coordinator_barrier.py`) ensuring `0%` CPU idle and instant wakeups. 
- **Exception for Virtual Time Polling**: `QmpBridge` still executes `asyncio.wait(..., timeout=0.1)` when querying QMP for virtual time because QEMU's `query-replay` does not natively emit asynchronous events upon time advancement. This is the optimal architecture given the constraint and is now thoroughly documented.
- **Exception for Rate Limiting**: In stress tests (e.g., `test_arch13_priority.py`), `asyncio.sleep(0.01)` is retained legitimately for traffic shaping and network flooding.

All constraints and corner-cases have been validated under ASan load scaling.
