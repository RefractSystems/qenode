# virtmcu Active Implementation Plan

**Goal**: VirtMCU turns QEMU into node Binary-Compatible Deterministic Simulation for Distributed Systems. It supports dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation.
  The software MUST be at the highest Enteprise Quality following the SOTA of software development.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

To maintain performance, type-safety, and long-term maintainability, the following language rules apply:

1. **Write in Rust if**:
   * It touches a virtual clock, routes a packet, or handles a bit-for-bit hardware register.
   * It is a complex generator or validation tool (e.g., parsing topologies to emit QEMU CLI args) where schema adherence is critical.
   * It is a high-performance adapter or bridge interfacing with external simulators (e.g., SystemC).
   * It handles test orchestration, monitoring, or CI verification.

2. **Avoid Bash for Orchestration**:
   * Bash is strictly for simple aliases, CI glue, or single-command wrappers.
   * Complex test setups involving background PIDs, inter-process communication, or fragile timing dependencies MUST be written in Rust (via `tokio`).

3. **Transport Abstraction Mandate** (all new and migrated code):
   * **`DataTransport` is the only API**: Peripheral and coordinator code MUST only call `DataTransport` trait methods (`publish`, `reserve`, `commit`). Which implementation is injected (`UdsDataTransport`, `ZenohDataTransport`, etc.) is a DI decision made at startup from topology config. Peripheral code has zero knowledge of the underlying transport.
   * **No direct transport construction in peripherals**: Direct `zenoh::Session` calls, `SafeSubscription`, or raw Zenoh publisher construction are BANNED in `hw/rust/`. Transport instances are injected, never constructed inside peripherals.
   * **Execution order**: We implement and validate `UdsDataTransport` first (simpler, no external process dependency). Zenoh is already implemented. The proof of abstraction correctness is that switching between them requires no peripheral code changes.
   * **All comms through coordinator**: Every inter-node and peripheral-to-peripheral link MUST route through `DeterministicCoordinator`. Raw peer-to-peer pub/sub between nodes is banned (RFC-0024). This applies to all media: UART, Ethernet, CAN-FD, 802.15.4, WiFi.

---

## [P0] Immediate Tactical Sprint: Module Loading, Safe Peripherals & Zero-Copy API

This section outlines our highest priority (P0) sequence of work. We have already completed the foundation (Steps 1-3: Assertion-Based Routing, Coordinator re-architecture, and Tracer Validation). The remaining critical path focuses on unblocking QEMU module loading, migrating all peripherals to 100% Safe Rust, and hooking them into the new Zero-Copy transport API.

**Explicit Dependency DAG:**
```text
 Task 1 (Module Loading) ────┐
                             ▼
 Task 3 (CoSimBridge) ───────► Task 4.1 (Framework) ──► Task 4.4 (Tracer Bullet) ──► Task 5 (Zero-Copy) ──► Task 4.6 (Mass Migrate)
                             ▲                                                                                    ▲
 Task 2 (SafeSubscription) ──┘                                                                                    │
                                                                                                                  │
 Task 10.1 (Unbounded Channels) & Task 10.3 (Thread Leaks) ───────────────────────────────────────────────────────┘

 Task 11.2 (UDS Backend) ──► Task 11.3 (Coordinator UDS Server) ──► Task 11.4 (chardev/netdev via coordinator) ──► Task 33 (Interactive Mode)
```
*Tasks 1, 2, and 3 are independent parallel tracks. Tasks 10.1 and 10.3 must land before Task 4.6 so migrated peripherals do not inherit those bugs. Task 5 (Phase 4) depends on the already-completed Phase 1 & 2 adapters, independent of Task 11 (Phase 3). Task 33 (External Input Boundaries) requires 11.3 (coordinator UDS server) to be complete first.*

### Task 1: Resolve QEMU Module Loading Blocker (The DTB Chicken-and-Egg)
**Goal:** Ensure `arm-generic-fdt` properly triggers QEMU's dynamic `.so` module auto-loader. Without this, no peripheral modules will load, causing Data Aborts.
*   **Actions:** 
- [x] 1.1: Patch `third_party/qemu/hw/arm/arm_generic_fdt.c` to call `module_load_qom(compat)` before attempting to instantiate a device.
- [x] 1.2: Add explicit "Fail Loudly" panic/exit logic in the FDT parser if a required VirtMCU module cannot be found.
- [x] 1.3: Ensure `xtask` correctly generates the module trigger maps (`modinfo.json`).
*   **Gate Criteria:** `reference_network.rs` integration tests pass without Data Aborts.

### Task 2: Resolve SafeSubscription Contradiction
**Goal:** Eliminate the architectural contradiction where `SafeSubscribe` is strictly banned, but is used internally by `DeterministicReceiver`.
*   **Actions:** 
- [x] 2.1: Refactor `DeterministicReceiver` off `SafeSubscription` entirely, OR formally scope the ban (e.g., allowed in framework internals, banned in peripheral code).
- [x] 2.2: Add explicit exception for backbone devices like `virtmcu-clock` to `GEMINI.md` (CLAUDE.md) ban scope and document inline with `virtmcu-allow`.
*   **Gate Criteria:** `DeterministicReceiver` is structurally sound without violating CLAUDE.md mandates.

### Task 3: CoSimBridge RAII IoC Refactor
**Status**: ✅ Completed.
**Goal**: Eliminate the manual, error-prone BQL-yielding and teardown boilerplate currently duplicated in `netdev`, `chardev`, and `actuator`. This is a strict prerequisite for declaring Task 4 completely "Safe".
*   **Actions**: 
- [x] 3.1: Refactor `hw/rust/comms/netdev`, `hw/rust/comms/chardev`, and `hw/rust/observability/actuator` to use `CoSimBridge`.
*   **Gate Criteria**: `CoSimBridge` handles vCPU registration, BQL-yielding wait, and teardown drain automatically across all bridges. Manual `VcpuCountGuard` / `Bql::temporary_unlock` boilerplate deleted.

### Task 4: "Zero Unsafe" Framework & Tracer Bullet Migration
**Goal:** Eradicate all `unsafe` from the peripheral layer. This happens in two architectural phases:
*   **Phase 1 (RFC-0023): "Zero Unsafe Boilerplate"**: Migrating peripherals to the `#[qom_device]` macro to generate the `unsafe extern "C"` FFI function boundaries automatically. This also implies adopting `DeterministicReceiver` to eliminate explicit `Bql` management.
*   **Phase 2 (RFC-0026): "Zero Unsafe Pointers"**: Pushing the remaining inner `unsafe {}` blocks down into the framework by introducing safe abstractions like `QomString`, dependency injection during `realize()`, and trampoline closures for callbacks.
*   **Actions:** 
- [x] 4.0: Implement RFC-0023 Safe QOM Macros (Phase 1-5: Safe DI, `Peripheral` trait, `#[qom_device]`, `class_init_custom` escape hatch, and DSO modular support).
- [x] 4.1: Implement RFC-0026 safe abstractions in the `virtmcu_qom` crate.
- [x] 4.2: Fix spin-yield loop in `MmioResult::Wait` macro to correctly use blocking `Condvar` instead of burning CPU and breaking determinism under backpressure. **Gate Criteria for 4.2:** Add a test proving `MmioResult::Wait` blocks the calling thread via condvar rather than spinning, verified under Miri.
- [x] 4.3: Finalize `reference-peripheral` template: audit and remove debug artifacts (e.g., `sim_err!`) to prevent cargo-culting.
- [ ] 4.4: **Tracer Bullet**: Migrate *only* the `reference-peripheral` through Phase 1, Phase 2, and the Zero-Copy API (Task 5) to validate the pipeline end-to-end.
- [ ] 4.5: Peripheral coverage audit: Enumerate all peripherals and classify test coverage (tested / untested / partially tested) before mass migration.
- [ ] 4.6: Mass migrate all remaining C-FFI peripherals in `observability/`, `mcu/`, and `comms/` to utilize these safe APIs. Must ensure every migrated peripheral is covered by a test before merging.
- [ ] 4.7: Enforce ASAN/TSAN/Miri shutdown integration tests for all newly migrated peripherals to guarantee teardown is sanitizer-clean.
*   **Gate Criteria:** `grep -r "unsafe" hw/rust/comms hw/rust/mcu hw/rust/observability` returns zero matches. All tests pass.

### Task 5: Peripheral Refactoring for Zero-Copy (RFC-0025 Phase 4)
**Goal:** Now that peripherals are fully safe, migrate them to the zero-allocation transport API. Note: This relies on the adapters completed in Phase 1 & 2 and is independent of the pure UDS backend in Task 11.
*   **Actions:** 
- [ ] 5.1: Update all `hw/rust/` peripherals to use the new `reserve()`/`commit()` API instead of the legacy `publish()` method. Remove all `encode_frame` boilerplate. (Start with Tracer Bullet in 4.4).
*   **Gate Criteria:** Framework compiles cleanly. `make test-check` and `make ci-full` pass. Additionally, at least one vendor firmware binary (e.g., from Task 24) runs successfully to confirm the API change doesn't silently break simulation output framing and timing.

---

## [P1] Quality, Hardening & Infrastructure

This section groups all architectural hardening, CI/CD improvements, and core simulation infrastructure tasks.

### Task 10: Core Hardening Roadmap — Stability & Security
**Status**: 🚧 Under Construction.
**Goal**: Systematically address known vulnerabilities regarding virtual time synchronization, the Big QEMU Lock (BQL), and high-frequency serialization.
**Tasks**:
- [ ] 10.1: Unbounded Channel Flooding: Implement `bounded(65536)` in `chardev` and `netdev`. Uphold the "Fail Loudly" mandate by using `.expect("FATAL: Channel flooded. PDES barrier failure.")` on send operations (instead of the banned `panic!` macro or silent telemetry drops).
- [ ] 10.2: Global Instance Singletons: Migrate `GLOBAL_CLOCK` and `GLOBAL_TELEMETRY` to use `VIRTMCU_EXPORT`-based registration from the QEMU main binary, avoiding the DSO Boundary Isolation Trap caused by Rust statistics.
- [ ] 10.3: Thread Leakage on Finalization: Implement `Arc<AtomicBool>` shutdown signals for background heartbeat and Zenoh subscriber threads to prevent pointer dereference after hot-unplug.
- [ ] 10.4: Startup Blocking: Implement configurable `VIRTMCU_ZENOH_CONNECT_TIMEOUT_MS` to prevent router discovery from blocking QEMU main thread for 4 seconds.
- [ ] 10.5: Serialization Alignment: Fully transition all core I/O to FlatBuffers (`vproto`) accessor patterns, removing manual `read_unaligned` and raw casts.

### Task 11: Native IPC Hybrid Architecture (RFC-0019 & RFC-0025)
**Status**: 🟡 Open.
**Goal**: `DataTransport` is the sole API. Which implementation runs is a DI decision made at startup from the topology config — peripheral code has zero knowledge of the underlying transport. We implement `UdsDataTransport` first because it is simpler to build and test; `ZenohDataTransport` already exists. The proof that the abstraction held: swapping implementations requires no peripheral code changes. All node↔coordinator links use a `DataTransport` impl; multi-host coordinator bridging is a separate concern.
**Tasks**:
- [x] 11.1: Zero-Copy API Definition & Adapter Arenas (RFC-0025 Phase 1 & 2): Introduce `TransportReservation` and update `virtmcu_api::DataTransport`. Update `ZenohDataTransport` to implement `reserve()`.
- [ ] 11.2: UDS Thread-Local Arena Backend (RFC-0025 Phase 3): Build the `UdsDataTransport` using thread-local arenas. `commit()` issues the `write()` syscall to the `DeterministicCoordinator`.
- [ ] 11.3: UDS Coordinator Server (`DeterministicCoordinator`): Extend `DeterministicCoordinator` to spin up a UDS server socket. Coordinator becomes the sole router: it owns both ends of every `sim/link/…` topic and enforces the quantum barrier before forwarding. No node speaks directly to another node.
- [ ] 11.4: Migrate `chardev` and `netdev` to coordinator-mediated routing: remove the raw Zenoh TX pub/sub path entirely. TX publishes to the coordinator via `UdsDataTransport`; RX uses `DeterministicReceiver` as today. `CoSimBridge` is retained only for true co-simulation boundary devices (`mmio-socket-bridge`, `remote-port`).
- [ ] 11.5: Time Authority UDS Integration
- [ ] 11.6: End-to-End Validation (`virtmcu-test-runner`): Validate the single-host topology using a multi-node deployment via the `virtmcu-test-runner` ensuring zero `vtime_ns` regressions. Transport impl is injected via DI; test verifies bit-identical output is independent of which `DataTransport` impl is configured.

### Task 12: Deep Oxidization & Testing Overhaul
*Ongoing*
**Tasks**:
- [ ] 12.1: Comprehensive Firmware Coverage (drcov integration). **Gate Criteria**: CI pipeline includes a `drcov` coverage step that fails the build if peripheral firmware coverage drops below an established baseline (e.g., 80%). (This is the #1 invariant CI gate).
- [ ] 12.3: Implement Rust `systemc-adapter` tool (C++ to Rust migration).
- [ ] 12.4: Unified Coverage Reporting (Host + Guest).
- [ ] 12.6: Migrate fragile Bash test orchestration scripts to a robust Rust test runner.

### Task 14: Execution Pacing & Faster-Than-Real-Time (FTRT) Support
**Status**: 🟡 Open.
**Goal**: Formalize the separation between **Wall-Clock Timeouts** and **Simulation Pacing**.
**Tasks**:
- [ ] 14.1: Host Timeout Scale
- [ ] 14.2: Coordinator Pacing (`--pacing <float>`)
- [ ] 14.3: Runtime UI Control
- [ ] 14.4: FTRT Proof Test

### Task 15: Document and Measure Simulation Frequency Ceiling
**Status**: 🟡 Open.
**Goal**: Document the maximum sustainable quantum rate for each transport option.
**Tasks**:
- [ ] 15.1: Create `tools/benchmarks/src/clock_rtt_bench.rs`
- [ ] 15.2: Add "Simulation Frequency Ceiling" table to ARCHITECTURE.md

---

## [P2] Hardware Expansion & Peripherals

This section tracks the addition of new peripherals, hardware co-processors, and vendor fidelity validation.

### Task 20: CAN-FD (Bosch M_CAN)
**Note**: Frame delivery MUST use coordinator-mediated routing (Task 11). No raw Zenoh pub/sub between nodes.
**Tasks**:
- [ ] 20.1: Implement missing Bosch M_CAN register logic.
- [ ] 20.2: Enable and verify CAN-FD frame payload delivery through `DeterministicCoordinator` via `DataTransport`.
- [ ] 20.3: Pass Vendor SDK loopback/echo tests.

### Task 21: FlexRay (Automotive)
**Tasks**:
- [ ] 21.1: Add FlexRay Interrupts (IRQ lines).
- [ ] 21.2: Implement Bosch E-Ray Message RAM Partitioning.
- [ ] 21.3: Fix SystemC build regression (CMake 4.3.1 compatibility).

### Task 22: WiFi (802.11)
**Note**: Frame delivery routes through `DeterministicCoordinator`. Radio propagation modelling (attenuation, RSSI, loss) lives in an "Ether/Physics Gateway" federate — NOT in the peripheral. The peripheral is medium-agnostic.
**Tasks**:
- [ ] 22.1: Harden `arm-generic-fdt` Bus Assignment (Child node auto-discovery).
- [ ] 22.2: Formalize `wifi` Rust QOM Proxy.
- [ ] 22.3: Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### Task 23: Thread Protocol
*Depends on: Task 22 (WiFi)*
**Tasks**:
- [ ] 23.1: Deterministic Multi-Node UART Bus Bridge.
- [ ] 23.2: SPI 802.15.4 Co-Processor (e.g., AT86RF233).

### Task 24: Vendor Firmware Validation (Binary Fidelity)
**Status**: 🟡 Open.
**Goal**: Validate VirtMCU against official, unmodified vendor SDK binaries targeting specific hardware.
**Tasks**:
- [ ] 24.1: CAN-FD (Bosch M_CAN)
- [ ] 24.2: Ethernet (MAC)

### Task 25: Connectivity Expansion
**Tasks**:
- [ ] 25.1: Bluetooth (nRF52840 RADIO emulation).
- [ ] 25.2: Automotive Ethernet (100BASE-T1).
- [ ] 25.3: Full Digital Twin (Multi-Medium Coordination).

---

## [P3] Strategic Evolution (Cool-to-Have & Roadmaps)

This section contains long-term, visionary features, UX improvements, and overarching architecture shifts.

### Task 30: Strategic Evolution: Enterprise SOTA SSoT (The Roadmap)
**Goal**: Transition from our current manual SSoT hacks to the idealized "Generation-Centric" workflow.
**Tasks**:
- [ ] 30.1: Phase 1: DRYing the World (Composition)
- [ ] 30.2: Phase 2: The IDL Bridge (TypeSpec)
- [ ] 30.3: Phase 3: USD Migration (OpenUSD)
- [ ] 30.4: Create `virtmcu-cli platform generate` to convert the modern YAML schema to DTB.
- [ ] 30.5: Update `target/release/virtmcu-run` to add `--yaml` support.

### Task 31: Real-Time Visualization & UI Framework
**Goal**: Implement a visually rich, interactive dashboard to visualize the simulation topology, link states, and live packet movement.
**Tasks**:
- [ ] 31.1: Simulation Gateway Pattern (Rust/Axum backend).
- [ ] 31.2: Transport Agnostic Observer.
- [ ] 31.3: Frontend (React Flow for topology graph).
- [ ] 31.4: AI Integration (MCP).

### Task 32: Enterprise Sensor Data Replay & Telemetry
**Goal**: Replace legacy RESD with MCAP/ASAM MDF4.
**Tasks**:
- [ ] 32.1: Add `mcap` to the workspace dependencies.
- [ ] 32.2: Build `virtmcu-replay` as a deterministic co-simulation node.
- [ ] 32.3: `mdf2mcap` Converter
- [ ] 32.4: OSI Support
- [ ] 32.5: Update `world_schema.json` to allow nodes to declare a `replay_trace: "file.mcap"` property.
- [ ] 32.6: Deprecate RESD

### Task 33: External Input Boundaries & Interactive Mode
**Status**: 🟡 Open.
**Depends on**: Task 11.3 (coordinator UDS server must exist before adding interactive endpoints).
**Goal**: Make the `DeterministicCoordinator` the sole point where wall-clock events (human input, external test harnesses) are converted into virtual-time events. Every link in the topology declares a `boundary` type; the coordinator enforces it at startup. This closes the determinism gap for human-in-the-loop use cases and gives replay for free.

**Key decisions**:
- All links route through the coordinator — no exceptions. The boundary type determines *how* the external end connects, not whether the coordinator is involved.
- Interactive mode runs vtime at 1:1 wall-clock ratio (existing `slaved-suspend` mechanism). The coordinator stamps arriving bytes with `delivery_vtime_ns = current_quantum_start` and dispatches them through the normal barrier path. From the simulation's perspective, human input is indistinguishable from any other coordinator message.
- Input log = coordinator records every external message (topic, vtime, payload) to MCAP (RFC-0017 / Task 32). Replay = feed the log back instead of the live socket. Same topology + same log → bit-identical output.

**Tasks**:
- [ ] 33.1: Add `boundary` field to topology YAML schema. Valid values: `coordinator` (default, quantum-barrier-enforced), `interactive` (real-time 1:1, live UDS endpoint), `replay` (headless, MCAP log fed by `virtmcu-replay`). Coordinator panics at startup if field is missing or unknown.
- [ ] 33.2: Coordinator external input endpoint: for each `boundary: interactive` link, open a named UDS socket at `/run/virtmcu/<sim_id>/<link_name>.sock`. Host processes (terminal emulators, test harnesses) connect and write raw bytes.
- [ ] 33.3: vtime stamping: bytes arriving between quantum steps are stamped with `current_quantum_start` and enqueued as coordinator messages for dispatch in the current quantum. Bytes arriving after quantum end are held and stamped with the next quantum start (at most one quantum of added latency, typically ≤ 1 ms).
- [ ] 33.4: Input log recording: append every external message to the session MCAP file before dispatch. Connects to Task 32.2 (`virtmcu-replay` node).
- [ ] 33.5: Replay mode: coordinator reads MCAP chunks and re-injects messages at their recorded `delivery_vtime_ns` via the `virtmcu-replay` node (Task 32.2). Simulation runs at unlimited speed (headless CI).
- [ ] 33.6: Interactive mode rate control: coordinator enforces vtime/wall-clock ratio = 1.0 when any interactive link is active. Connects to Task 14.2 (`--pacing` flag).

*Gate Criteria*: An interactive session (human types over chardev) produces an MCAP log. Running in replay mode with that log reproduces bit-identical QEMU output. Verified by `virtmcu-test-runner`.

---

## 4. Ongoing Risks (Watch List)

Items here have no immediate action — they are structural constraints or future triggers to monitor.

| ID | Risk | Status / Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Ongoing. QEMU version is pinned; all patches go through `cargo run -p virtmcu-cli -- setup patch-qemu`. Track upstream `accel/tcg` changes on each QEMU bump. |
| R7 | `icount` performance | Design guideline: use `slaved-icount` only when sub-quantum timing precision is required. `slaved-suspend` is the default. |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov`/coverage CI gate. Tracked as Task 12.1. |