# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation. The software MUST be at the highest Enteprise Quality following the SOTA of software development.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

## 1. General Guidelines & Mandates

### Milestone Lifecycle
Once a Milestone is completed and verified, it MUST be moved from `PLAN.md` to the `/docs/guide/05-project-history.md` file to maintain a clean roadmap and a clear historical record.

### Educational Content (Tutorials)
For every completed milestone, a corresponding tutorial lesson MUST be added in `/tutorial`.
- **Target**: CS graduate students and engineers.
- **Style**: Explain terminology, provide reproducible code, and teach practical debugging skills.

### Regression Testing
For every completed milestone, an automated integration test MUST be added to `tests/` or `tests/fixtures/guest_apps/`.
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

---



**Determinism migration (new — highest correctness priority):**
1. **DET-9** — Wireshark extcap plugin (Low priority / Redundant with INFRA-7).

**Hardware / infrastructure (existing, continue in parallel with DET work):**
2. **Milestone 27** — FlexRay IRQs + Bosch E-Ray Message RAM.
3. **Milestones 21 / 22** — WiFi / Thread Protocol expansion.
4. **Milestone 30.9 + 30.9.1** — Rust systemc-adapter + stress-adapter.
5. **Milestone 30.8 + 30.10** — Firmware coverage (drcov) + unified reporting.
6. **P12** — Deterministic Deadlock Detection (virtual-time budgets).
7. **Milestone 32** — Vendor Firmware Validation (Ethernet & CAN-FD Binary Fidelity).

---

### [Hardware] Milestone 24 — CAN-FD (Bosch M_CAN) 🚧
*Depends on: Milestone 19 (Rust QOM) ✅*
- [ ] **24.1** Implement missing Bosch M_CAN register logic.
- [ ] **24.2** Enable and verify CAN-FD frame payload delivery over Zenoh.
- [ ] **24.3** Pass Vendor SDK loopback/echo tests (Link to Milestone 32.1).

### [Hardware] Milestone 27 — FlexRay (Automotive) 🚧
*Depends on: Milestone 5 (Bridge) ✅, Milestone 19 (Rust QOM) ✅*
- [ ] **27.1.1** Add FlexRay Interrupts (IRQ lines).
- [ ] **27.1.2** Implement Bosch E-Ray Message RAM Partitioning.
- [ ] **27.2.1** Fix SystemC build regression (CMake 4.3.1 compatibility).

### [Hardware] Milestone 21 — WiFi (802.11) 🚧
*Depends on: Milestone 20.5 (SPI)*
- [ ] **21.7.1** Harden `arm-generic-fdt` Bus Assignment (Child node auto-discovery).
- [ ] **21.7.2** Formalize `wifi` Rust QOM Proxy.
- [ ] **21.2** Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### [Hardware] Milestone 22 — Thread Protocol 🚧
*Depends on: Milestone 20.5 (SPI), Milestone 21 (WiFi)*
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

### **[Infrastructure] Milestone 30 — Deep Oxidization & Testing Overhaul** 🚧
*Ongoing*
- [ ] **30.8** Comprehensive Firmware Coverage (drcov integration).
- [x] **30.9.1** Implement Rust `stress-adapter` tool.
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).


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
- [ ] **32.3** **Provenance Enforcement**: Update `tests/firmware/*/PROVENANCE.md` (and create for all new firmwares) to mandate that *all* test firmwares explicitly list the exact real-world MCU, the specific peripheral name (e.g., "NXP S32K144 LPUART0"), the vendor SDK version, and a reproducible download/build link.


### [Infrastructure] INFRA-9 — Execution Pacing & Faster-Than-Real-Time (FTRT) Support
**Status**: 🟡 Open.
**Goal**: Formalize the separation between **Wall-Clock Timeouts** (fail-fast boundaries) and **Simulation Pacing** (controlling guest execution speed relative to reality). VirtMCU must run FTRT in CI, but support interactive real-time (1.0x) or slow-motion (e.g., 10.0x) for human-in-the-loop UI and GDB sessions.
**What needs to be improved**: Tests and runtime environments currently assume "as fast as possible." When connecting a frontend UI or Wireshark, the simulation runs too fast for human observation. Conversely, we must mathematically prove that CI runs FTRT without artificial framework bottlenecks.
**How it's tested**: 
1. **Host Timeout Scale**: Implement `HOST_TIMEOUT_MULTIPLIER` in `conftest_core.py` to transparently stretch/shrink wait boundaries based on ASan/CI runners.
2. **Coordinator Pacing**: Add `--pacing <float>` to `deterministic_coordinator`. `0.0` = FTRT (default), `1.0` = Real-time, `10.0` = Slow motion.
3. **Runtime UI Control**: Expose a QMP/Zenoh endpoint allowing a connected Frontend UI to dynamically adjust the pacing multiplier at runtime.
4. **FTRT Proof Test**: Create a CI test that simulates 60 seconds of virtual stress-load, asserting that it completes in `< 5 seconds` of Wall-Clock time.

### [Future] Real-Time Visualization & UI Framework

**Goal**: Implement a visually rich, interactive dashboard to visualize the simulation topology, link states, and live packet movement.

**Design Mandates**:
1.  **Simulation Gateway Pattern**: Use a **FastAPI** backend as an "Intelligence Gateway" to aggregate raw simulation data and serve it to both humans (WebSockets) and AI Agents (REST).
2.  **Transport Agnostic Observer**:
    *   **Zenoh**: Passive subscriber to `sim/**` topics.
    *   **Unix Sockets**: Implement an "Observer Port" in the `deterministic_coordinator` that "tees" all routed traffic to a local Unix stream.
3.  **Frontend**: Use **React Flow** for the topology graph. Packets should be animated as glowing CSS markers traveling along SVG edge paths based on live `(src, dst, proto)` events.
4.  **AI Integration (MCP)**: The Gateway must provide semantic aggregation (e.g., `/api/network/stats`) to prevent overwhelming AI agent context windows with raw packet data.

### [Future] Connectivity Expansion
- [ ] **Milestone 23**: Bluetooth (nRF52840 RADIO emulation).
- [ ] **Milestone 26**: Automotive Ethernet (100BASE-T1).
- [ ] **Milestone 28**: Full Digital Twin (Multi-Medium Coordination).
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

## 4. Ongoing Risks (Watch List)

Items here have no immediate action — they are structural constraints or future triggers to monitor.

| ID | Risk | Status / Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Ongoing. QEMU version is pinned; all patches go through `scripts/apply-qemu-patches.sh`. Track upstream `accel/tcg` changes on each QEMU bump. |
| R7 | `icount` performance | Design guideline: use `slaved-icount` only when sub-quantum timing precision is required. `slaved-suspend` is the default. |
| R11 | Zenoh session deadlocks in teardown | Mitigated: `SafeSubscriber` (Milestone 1) and BQL-yielding (Milestone 18.7). |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov`/coverage CI gate. Tracked as Milestone 30.8. |

## 5. Permanently Rejected / Won't Do
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
- [x] Fixed Miri tests across the workspace
