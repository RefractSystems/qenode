# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation.
  The software MUST be at the highest Enteprise Quality following the SOTA of software development.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

---

## Language Selection Rules (Enterprise SOTA Mandate)
To maintain performance, type-safety, and long-term maintainability, the following language rules apply:

1. **Write in Rust if**:
   * It touches a virtual clock, routes a packet, or handles a bit-for-bit hardware register.
   * It is a complex generator or validation tool (e.g., parsing topologies to emit QEMU CLI args) where schema adherence is critical.
   * It is a high-performance adapter or bridge interfacing with external simulators (e.g., SystemC).
   * It handles test orchestration, monitoring, or CI verification.

2. **Avoid Bash for Orchestration**:
   * Bash is strictly for simple aliases, CI glue, or single-command wrappers.
   * Complex test setups involving background PIDs, inter-process communication, or fragile timing dependencies MUST be written in Rust (via `tokio`).

---


**Hardware / infrastructure (existing, continue in parallel with DET work):**
3. **Milestone 27** — FlexRay IRQs + Bosch E-Ray Message RAM.
4. **Milestones 21 / 22** — WiFi / Thread Protocol expansion.
5. **Milestone 30.9 + 30.9.1** — Rust systemc-adapter + stress-adapter.
6. **Milestone 30.8 + 30.10** — Firmware coverage (drcov) + unified reporting.
7. **P12** — Deterministic Deadlock Detection (virtual-time budgets).
8. **Milestone 32** — Vendor Firmware Validation (Ethernet & CAN-FD Binary Fidelity).
9. **Milestone 33** — Deprecation of `repl2qemu` and `.repl` format (Migration to YAML+SVD SSOT).

---

### Phase X: The Native Rust Singularity (Testing Framework Migration) ✅
**Status**: ✅ Completed.
**Goal**: Eradicate `virtmcu-test-runner`, Python-based orchestration, and fragile bash wrappers, shifting 100% of the integration testing logic into native `#[tokio::test]` leveraging the RAII-safe `virtmcu-test-runner` library.

**Tasks**:
- [x] **X.1 Core Tooling Migration**: Port QMP edge case and failure injection tests to `tests/native_integration/tests/qmp.rs`.
- [x] **X.2 Generic Peripheral Monitors**: Build Flatbuffer-aware type-safe Zenoh clients (`monitors.rs`) for Telemetry, SPI, LIN, FlexRay, UART, and Actuators.
- [x] **X.3 Python Purge - Peripherals**: Rewrite `test_spi.py`, `test_telemetry.py`, `test_canfd.py`, `test_lin.py`, `test_flexray.py`, `test_uart_echo.py`, and `test_actuator.py` into native Rust and delete the Python implementations.
- [x] **X.4 Python Purge - Infrastructure**: Migrate the core Determinism logic: `test_clock_suspend.py`, `test_ftrt_timing.py`, `test_coordinator.py`, `test_topology_integrity.py`.
- [x] **X.5 CLI Tool Subsumption**: Integrate `--coverage`, `--miri`, and `--asan` flags natively into the `virtmcu-test-runner` CLI, subsuming `scripts/testing/*.sh` wrappers.
- [x] **X.6 The Final Strike**: Delete `tools/testing/virtmcu_test_suite/`, remove `virtmcu-test-runner` from all environment files, delete all YAML specs in `tests/specs/`, and purge Python scripts embedded within `docs/tutorials/`. Migrated `yaml2qemu` to `packaging/virtmcu-tools` as a self-contained package.


---

### [Hardware] Milestone 24 — CAN-FD (Bosch M_CAN) 🚧
- [ ] **24.1** Implement missing Bosch M_CAN register logic.
- [ ] **24.2** Enable and verify CAN-FD frame payload delivery over Zenoh.
- [ ] **24.3** Pass Vendor SDK loopback/echo tests (Link to Milestone 32.1).

### [Hardware] Milestone 27 — FlexRay (Automotive) 🚧
- [ ] **27.1.1** Add FlexRay Interrupts (IRQ lines).
- [ ] **27.1.2** Implement Bosch E-Ray Message RAM Partitioning.
- [ ] **27.2.1** Fix SystemC build regression (CMake 4.3.1 compatibility).

### [Hardware] Milestone 21 — WiFi (802.11) 🚧
- [ ] **21.7.1** Harden `arm-generic-fdt` Bus Assignment (Child node auto-discovery).
- [ ] **21.7.2** Formalize `wifi` Rust QOM Proxy.
- [ ] **21.2** Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### [Hardware] Milestone 22 — Thread Protocol 🚧
*Depends on: Milestone 21 (WiFi)*
- [ ] **22.1** Deterministic Multi-Node UART Bus Bridge.
- [ ] **22.2** SPI 802.15.4 Co-Processor (e.g., AT86RF233).

### **[ARCH-21] CoSimBridge RAII IoC Refactor** — Architecture & Reliability

**Status**: 🚧 Under Construction (Completed for `mmio-socket-bridge` and `remote-port`).

**Goal**: Eliminate the manual, error-prone BQL-yielding and teardown boilerplate currently duplicated in `netdev`, `chardev`, and `actuator`. Move from a "Developer-must-remember" safety model to a "Safety-by-Construction" framework.

**Files to modify**:
- `hw/rust/comms/netdev/src/lib.rs` — Refactor to use `CoSimBridge`.
- `hw/rust/comms/chardev/src/lib.rs` — Refactor to use `CoSimBridge`.
- `hw/rust/observability/actuator/src/lib.rs` — Refactor to use `CoSimBridge`.

**Definition of Done**:
- [ ] `CoSimBridge` handles vCPU registration, BQL-yielding wait, and teardown drain automatically across all bridges.
- [ ] Manual `VcpuCountGuard` / `Bql::temporary_unlock` boilerplate deleted.
- [ ] Shutdown stress tests pass under ASan without UAF or hangs.

---

### **[ARCH-22] MmioDevice Trait & Condvar BQL Yielding** — Correctness & Safety
**Status**: ✅ Completed.

**Goal**: Eliminate simulation starvation bugs (livelock) caused by guest firmware tight-polling MMIO registers. Replace the manual `Bql::temporary_unlock()` + `yield_now()` pattern with a structurally safe, closure-based `MmioDevice` trait and `wait_yielding_bql`.

**Tasks**:
- [x] **Phase 1: True Blocking:** Update the existing stopgap `yield_now()` usages in `sensor` and `ieee802154` to use `QemuCond::wait_yielding_bql` triggered by their respective Zenoh background threads.
- [x] **Phase 2: Linter Enforcement:** Add `std::thread::yield_now()` to the custom `virtmcu-test-runner` linter `banned_patterns.rs` to prevent developers from manually spin-yielding in peripheral code.
- [x] **Phase 3: MmioDevice Macro:** Create a `pub trait MmioDevice` in `virtmcu-qom` that returns an `MmioResult` (or uses a `wait_for` closure pattern). Create a `#[derive(MmioDevice)]` proc-macro that generates the `unsafe extern "C"` MMIO callbacks and fully encapsulates the BQL condvar yielding logic.
- [x] **Phase 4: Migration:** Port all existing Rust peripherals (sensor, radio, actuator, etc.) to the new `MmioDevice` pattern and delete the manual C-FFI boilerplate. Update the `rust-dummy` template.

---

### **[Infrastructure] Milestone 30 — Deep Oxidization & Testing Overhaul** 🚧
*Ongoing*
- [ ] **30.8** Comprehensive Firmware Coverage (drcov integration).
- [x] **30.9.1** Implement Rust `stress-adapter` tool.
- [ ] **30.9.2** Implement Rust `systemc-adapter` tool (C++ to Rust migration).
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).
- [x] **30.11** Migrate `yaml2qemu.py` validation logic to Rust. This ensures strict, compile-time adherence to the TypeSpec schema via the Rust Domain Models. (Completed as part of `virtmcu-cli platform generate` port).
- [ ] **30.12** Migrate fragile Bash test orchestration scripts (e.g., in `tests/fixtures/guest_apps/irq_stress/`) to a robust Rust test runner.

### [Hardware] Milestone 32 — Vendor Firmware Validation (Binary Fidelity) 🚧
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
- [ ] **32.1** **CAN-FD (Bosch M_CAN)**: 
  - *Target*: Identify a specific vendor MCU with a Bosch M_CAN controller (e.g., STM32G4, NXP S32K3).
  - *Action*: Download the official vendor SDK CAN-FD example (e.g., echo/loopback). Compile unmodified and implement the missing M_CAN register logic in VirtMCU (Milestone 24) to make the vendor firmware pass.
- [ ] **32.2** **Ethernet (MAC)**:
  - *Target*: Identify a specific vendor MCU/Board with an Ethernet MAC supported by QEMU (e.g., SMSC LAN9118 on Cortex-A15, or NXP ENET on i.MX).
  - *Action*: Download the official vendor SDK lwIP/ping example. Compile unmodified and test against `virtmcu-netdev` to verify bidirectional packet flow.
- [x] **32.3** **Provenance Enforcement**: Update `tests/firmware/*/PROVENANCE.md` (and create for all new firmwares) to mandate that *all* test firmwares explicitly list the exact real-world MCU, the specific peripheral name (e.g., "NXP S32K144 LPUART0"), the vendor SDK version, and a reproducible download/build link.

### [Infrastructure] Milestone 33 — Deprecation of `repl2qemu` and `.repl` format 🚧
**Status**: 🚧 Completed (Legacy files purged).

**Goal**: Complete the transition to the bifurcated hardware description model (YAML for topology via OpenUSD + CMSIS-SVD for micro-architecture/registers). 

**Tasks**:
- [x] **33.1**: Migrate any remaining legacy `.repl` platforms in the `worlds/` directory to the new YAML format.
- [x] **33.2**: Purge legacy `repl2qemu` Python scripts and dependencies.
- [ ] **33.3**: Update any documentation guides (e.g., in `docs/guide/`) still referencing `.repl` files to exclusively describe the YAML + SVD workflow.


### [Infrastructure] INFRA-9 — Execution Pacing & Faster-Than-Real-Time (FTRT) Support
**Status**: 🟡 Open.
**Goal**: Formalize the separation between **Wall-Clock Timeouts** (fail-fast boundaries) and **Simulation Pacing** (controlling guest execution speed relative to reality). VirtMCU must run FTRT in CI, but support interactive real-time (1.0x) or slow-motion (e.g., 10.0x) for human-in-the-loop UI and GDB sessions.
**What needs to be improved**: Tests and runtime environments currently assume "as fast as possible." When connecting a frontend UI or Wireshark, the simulation runs too fast for human observation. Conversely, we must mathematically prove that CI runs FTRT without artificial framework bottlenecks.
**How it's tested**: 
1. **Host Timeout Scale**: Implemented logic to transparently stretch/shrink wait boundaries based on ASan/CI runners.
2. **Coordinator Pacing**: Add `--pacing <float>` to `deterministic_coordinator`. `0.0` = FTRT (default), `1.0` = Real-time, `10.0` = Slow motion.
3. **Runtime UI Control**: Expose a QMP/Zenoh endpoint allowing a connected Frontend UI to dynamically adjust the pacing multiplier at runtime.
4. **FTRT Proof Test**: Create a CI test that simulates 60 seconds of virtual stress-load, asserting that it completes in `< 5 seconds` of Wall-Clock time.

### [Future] Real-Time Visualization & UI Framework

**Goal**: Implement a visually rich, interactive dashboard to visualize the simulation topology, link states, and live packet movement.

**Design Mandates**:
1.  **Simulation Gateway Pattern**: Use a **Rust/Axum** backend as an "Intelligence Gateway" to aggregate raw simulation data and serve it to both humans (WebSockets) and AI Agents (REST). This ensures performance (FTRT traffic teeing) adheres to the project's language selection rules.
2.  **Transport Agnostic Observer**:
    *   **Zenoh**: Passive subscriber to `sim/**` topics.
    *   **Unix Sockets**: Implement an "Observer Port" in the `deterministic_coordinator` that "tees" all routed traffic to a local Unix stream.
3.  **Frontend**: Use **React Flow** for the topology graph. Packets should be animated as glowing CSS markers traveling along SVG edge paths based on live `(src, dst, proto)` events.
4.  **AI Integration (MCP)**: The Gateway must provide semantic aggregation (e.g., `/api/network/stats`) to prevent overwhelming AI agent context windows with raw packet data.

### [Future] Connectivity Expansion
- [ ] **Milestone 23**: Bluetooth (nRF52840 RADIO emulation).
- [ ] **Milestone 26**: Automotive Ethernet (100BASE-T1).
- [ ] **Milestone 28**: Full Digital Twin (Multi-Medium Coordination).


### **[ARCH-14] Document and Measure Simulation Frequency Ceiling** — Observability

**Status**: 🟡 Open. Depends on: DET-4 (Unix socket transport).

**Goal**: Document the maximum sustainable quantum rate for each transport option.
Add a benchmark script. Add the measured results as a table in ARCHITECTURE.md so
engineers can choose the right transport for their scenario.

**Files to create**:
- `tools/benchmarks/src/clock_rtt_bench.rs` — measures median clock RTT across 10 000 quanta

**Files to modify**:
- `docs/architecture/01-system-overview.md` — add "Simulation Frequency Ceiling" table

**Definition of Done**:
- [ ] Benchmark tool exists and is runnable in CI.
- [ ] Results table added to ARCHITECTURE.md §9.

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
- [ ] `make test-lint` passes.

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
- [ ] `make test-lint` passes.

---

## 4. Ongoing Risks (Watch List)

Items here have no immediate action — they are structural constraints or future triggers to monitor.

| ID | Risk | Status / Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Ongoing. QEMU version is pinned; all patches go through `cargo run -p virtmcu-cli -- setup patch-qemu`. Track upstream `accel/tcg` changes on each QEMU bump. |
| R7 | `icount` performance | Design guideline: use `slaved-icount` only when sub-quantum timing precision is required. `slaved-suspend` is the default. |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov`/coverage CI gate. Tracked as Milestone 30.8. |

---

### Phase 1: Build System Oxidation (Makefile -> xtask) ✅
**Status**: ✅ Completed.
**Goal**: Centralize and strongly-type the complex Bash/Make build logic into a Rust `xtask` crate to improve maintainability and cross-platform reliability.

**Tasks**:
- [x] Create the `xtask` workspace member.
- [x] Port `BUILD_DEPS` parsing, version sync, and image tagging logic to Rust.
- [x] Implement subcommands for Docker builds, QEMU compilation, and test execution.
- [x] Refactor the root `Makefile` into a thin wrapper delegating to `cargo xtask`.
- [x] Update documentation (`01-build-system.md`).

---

### Phase 2: Enterprise Sensor Data Replay & Telemetry
**Status**: 🟡 Open.
**Goal**: Implement ADR-017. Replace the legacy RESD format with a SOTA Hybrid Replay Architecture (MCAP and ASAM MDF4) to enable Enterprise Grade Sensor-in-the-Loop and Hardware-in-the-Loop co-simulation.

**Tasks**:
- [ ] **2.1 Dependency Update**: Add `mcap` and `camino` (or equivalent path handler) crates to the VirtMCU workspace.
- [ ] **2.2 `virtmcu-replay` Node**: Create a new Zenoh client binary (`tools/virtmcu-replay`) that acts as a Deterministic Co-Simulation node. It must participate in the CMB quantum barrier and synchronize MCAP payload injection to `delivery_vtime_ns`.
- [ ] **2.3 `mdf2mcap` Converter**: Build a CLI tool/adapter to convert Automotive ASAM MDF4 (`.mf4`) traces into VirtMCU-compatible MCAP files.
- [ ] **2.4 OSI Support**: Integrate Protobuf definitions for ASAM OSI (Open Simulation Interface) into the schema pipeline to support Object-Level sensor injection.
- [ ] **2.5 Schema Update**: Update `world_schema.json` and `yaml2qemu` to support declaring a `replay_trace: "file.mcap"` property on peripherals/nodes.
- [ ] **2.6 Deprecate RESD**: Remove any residual Renode RESD parsing logic and update documentation to reflect the new MCAP standard.

