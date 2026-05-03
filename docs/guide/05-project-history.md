# VirtMCU Completed Milestones

This file serves as a historical record of completed milestones and tasks in the VirtMCU project.

---

## Simulation Stability & FlexRay Support (May 2026) ✅

**Status**: Done

### Tasks
- [x] **FlexRay Support**: Implemented native Rust FlexRay QOM peripheral with Bosch E-Ray Message RAM support.
- [x] **Lookahead Barrier**: Refactored `DeterministicCoordinator` and `QuantumBarrier` to support arbitrary lookahead via `HashMap<u64, QuantumData>`, allowing nodes to pre-calculate future quanta.
- [x] **Zenoh Session Isolation**: Enforced strict client-mode isolation for Zenoh sessions in tests via `make_client_config()` and `lint-python` gates.
- [x] **Deterministic Routing Sync**: Implemented `ensure_session_routing()` to eliminate races during Zenoh declaration propagation.
- [x] **Harden Test Suites**: Simplified smoke test domains and moved legacy Bash tests to structured `pytest` orchestration.

---

## Architectural Hardening — Definitive Race Condition Fixes (April 2026) ✅

**Status**: Done

### Tasks
- [x] **Hardened Synchronization Barrier**: Identified and fixed a non-deterministic race where fast nodes could finish quantum N+1 before the coordinator processed N.
- [x] **Unified State Lock**: Refactored `QuantumBarrier` to use a single `Mutex<BarrierState>` for all atomic transitions, eliminating deadlock risks.
- [x] **Lookahead Buffer**: Implemented 1-quantum lookahead in the barrier to safely buffer fast-arriving messages without manual resets.
- [x] **Stress Testing**: Added `barrier_race_stress.rs` integration test, verified with 100x local loop stability.
- [x] **DTC Hardening**: Updated FDT emitter to treat DTC warnings as non-fatal to ensure CI stability.
- [x] **Wireless Fix**: Updated wireless tests for modern `name@address` node naming convention.

---

## Architectural Hardening — ASan & UAF Prevention ✅

**Status**: Done

### Tasks
- [x] **Milestone 1**: `SafeSubscriber` RAII wrapper implemented in `transport-zenoh` to automatically manage BQL and teardown races.
- [x] **Milestone 2**: `QomTimer` RAII wrapper implemented in `virtmcu-qom` for automated timer destruction.
- [x] **Milestone 3**: All Rust peripherals refactored to use the safe wrappers.

---

## Milestone 0 — Repository Setup ✅

**Status**: Done

### Tasks
- [x] Directory scaffold, `CLAUDE.md`, `PLAN.md`, `README.md`, `docs/ARCHITECTURE.md`, `.gitignore`.

---

## Milestone 1 — QEMU Build with arm-generic-fdt ✅

### Tasks
- [x] **1.1** `scripts/setup-qemu.sh`.
- [x] **1.2** Minimal `tests/fixtures/guest_apps/boot_arm/minimal.dts`.
- [x] **1.3** `scripts/run.sh` skeleton.
- [x] **1.4** Smoke-test: boot minimal DTB.
- [x] **1.5** Tutorial lesson 1.

---

## Milestone 2 — Dynamic QOM Plugin Infrastructure ✅

### Tasks
- [x] **2.1** `hw/rust/common/rust-dummy`.
- [x] **2.2** Update QEMU module build configuration.
- [x] **2.3** Verify native module loading.
- [x] **2.4** Rust template.
- [x] **2.5** Tutorial lesson 2.

---

## Milestone 3 — repl2qemu Parser ✅

### Tasks
- [x] **3.1** Obtain reference `.repl` files.
- [x] **3.2** `tools/repl2qemu/parser.py`.
- [x] **3.3** `tools/repl2qemu/fdt_emitter.py`.
- [x] **3.4** `tools/repl2qemu/cli_generator.py`.
- [x] **3.5** `tools/repl2qemu/__main__.py`.
- [x] **3.6** Unit tests.
- [x] **3.7** Tutorial lesson 3.
- [x] **3.8** Integration test `tests/fixtures/guest_apps/yaml_boot/smoke_test.sh`.

---

## Milestone 5 — Co-Simulation Bridge ✅

### Tasks
- [x] **5.1** `hw/rust/mmio-socket-bridge` and `tools/systemc_adapter/`.
- [x] **5.4** Path A vs B vs C guide.
- [x] **5.5** Tutorial lesson 5.
- [x] **5.6** mmio-socket-bridge resilience.
- [x] **5.7** MMIO Stress Test.
- [x] **5.8** Bridge Reconnection Hardening.

---

## Milestone 8 — Interactive and Multi-Node Serial (UART) ✅

### Tasks
- [x] **8.1** Echo Firmware.
- [x] **8.2** Tutorial Lesson 8.
- [x] **8.3** Deterministic Zenoh Chardev.
- [x] **8.4** Multi-Node UART Test.
- [x] **8.5** Fix `libc::malloc` issues.
- [x] **8.6** High-Baud Stress Test.

---

## Milestone 9 — Advanced Co-Simulation: Shared Media (SystemC) ✅

### Tasks
- [x] **9.1** Asynchronous IRQ Protocol.
- [x] **9.2** Multi-threaded SystemC Adapter.
- [x] **9.3** Educational CAN Model.
- [x] **9.4** Tutorial Lesson 9.

---

## Milestone 10 — Telemetry Injection & Physics Alignment (SAL/AAL) ✅

### Tasks
- [x] **10.1** SAL/AAL Interfaces.
- [x] **10.2** RESD Engine.
- [x] **10.3** MuJoCo Bridge.
- [x] **10.4** OpenUSD Tool.
- [x] **10.5** Tutorial Lesson 10.
- [x] **10.6** Native Zenoh Actuators.

---

## Milestone 11 — RISC-V Expansion & Framework Maturation ✅

### Tasks
- [x] **11.1** RISC-V Machine Generation.
- [x] **11.2** Virtual-Time Timeouts.
- [x] **11.3** Remote Port (Path B).
- [x] **11.4** FirmwareStudio Migration.

---

## Milestone 12 — Advanced Observability & Interactive APIs ✅

### Tasks
- [x] **12.1** Telemetry Tracing.
- [x] **12.2** Dynamic Network Topology API.
- [x] **12.3** Standardized UI Topics.
- [x] **12.4** Tutorial Lesson 12.
- [x] **12.5** Concurrency inside `irq_slots`.
- [x] **12.6** FlatBuffers rigidity.
- [x] **12.7** Safe QOM Path Resolution.
- [x] **12.8** Telemetry Throughput Benchmark.

---

## Milestone 13 — AI Debugging & MCP Interface ✅

### Tasks
- [x] **13.1** MCP Tools.
- [x] **13.2** Semantic Debugging API.
- [x] **13.3** Zenoh-MCP Bridge.
- [x] **13.4** Tutorial Lesson 13.

---

## Milestone 14 — Wireless & IoT RF Simulation (BLE, Thread, WiFi) ✅

### Tasks
- [x] **14.1** HCI over Zenoh.
- [x] **14.2** 802.15.4 MAC.
- [x] **14.3** RF Propagation.
- [x] **14.4** Tutorial Lesson 14.
- [x] **14.5** MAC State Machine.
- [x] **14.6** Coordinator Scaling.
- [x] **14.7** Dynamic Topology updates.
- [x] **14.8** RF Header schema.
- [x] **14.9** RF Assumption improvements.

---

## Milestone 15 — Distribution & Packaging ✅

### Tasks
- [x] **15.1** PyPI Package.
- [x] **15.2** Binary Releases.
- [x] **15.3** Tutorial Lesson 15.

---

## Milestone 16 — Performance & Determinism CI ✅

### Tasks
- [x] **16.1** IPS Benchmarking.
- [x] **16.2** Latency Tracking.
- [x] **16.3** Tutorial Lesson 16.
- [x] **16.4** Jitter Injection Test.
- [x] **16.5** Automated Trend Tracking.

---

## Milestone 17 — Security & Hardening (Fuzzing) ✅

### Tasks
- [x] **17.1** Network Boundary Fuzzing.
- [x] **17.2** Parser Fuzzing.
- [x] **17.3** Tutorial Lesson 17.

---

## Milestone 18 — Native Rust Zenoh Migration (Oxidization) ✅

### Tasks
- [x] **18.1** Enable QEMU Rust.
- [x] **18.2** Native Zenoh-Clock.
- [x] **18.3** Native Zenoh-Netdev.
- [x] **18.4** Native Zenoh-Telemetry.
- [x] **18.5** Native Chardev, Actuator, ieee802154, UI.
- [x] **18.6** Verification.
- [x] **18.7** Fix BQL.
- [x] **18.8** Fix telemetry return.
- [x] **18.9** adopt virtmcu-qom.
- [x] **18.10** adopt virtmcu-qom netdev.
- [x] **18.11** Align Cargo.toml.
- [x] **18.12** Zenoh session helper.
- [x] **18.13** FFI Safety & Audit.
- [x] **18.14** Lock-Free Priority Queue.

---

## Milestone 19 — Native Rust QOM API Migration ✅

### Tasks
- [x] **19.1** QOM type registration.
- [x] **19.2** Eliminate non-netdev C shims.
- [x] **19.3** Eliminate `netdev` C shim.
- [x] **19.4** Delete FFI shim.
- [x] **19.5** Memory Layout Verification.
- [x] **19.6** Refactor lint suppression.
- [x] **19.7** Rust QOM Stabilization.
- [x] **19.8** Rust QOM Jitter Fix.

---

## Milestone 20 — Shared Rust API Crate (`virtmcu-api`) ✅

### Tasks
- [x] **20.1** Create `virtmcu-api`.
- [x] **20.2** Refactor plugins.

---

## Milestone 25 — Local Interconnect Network (LIN) ✅

### Tasks
- [x] **25.1** LIN Controller.
- [x] **25.2** Synchronization.
- [x] **25.3** Firmware.
- [x] **25.4** Verification.

---

## Milestone 31 — Advanced CI & Build Pipeline Optimization ✅

### Tasks
- [x] **31.1** Ninja Lock Contention fix.
- [x] **31.2** Selective CI.
- [x] **31.3** Parallelization.
- [x] **31.4** C Static Analysis.
- [x] **31.5** LLVM Linker.
- [x] **31.6** Codespell.

---

## P0 Serial Tasks — Enterprise Hardening ✅

**Completed**: 2026-04-25 (A–I) / 2026-04-27 (P01 mechanism, P09 partial)

| Task | Description |
|---|---|
| P01 | BOOT_QUANTUM_TIMEOUT grace window (5 min). |
| P02 | ptr::read_unaligned for packed structs. |
| P04 | `BqlGuarded<T>` migration. |
| P05 | Pure Rust Mutex/Condvar conversion. |
| P06 | VcpuCountGuard RAII. |
| P07 | Zero untagged sleeps in hw/rust/. |
| P08 | ci-asan.yml. |
| P09 | clippy -D warnings. |
| P10 | Chardev flow control & VirtualTimeAuthority fixtures. |
| Task F | vproto.py FlatBuffers migration. |

---

## 2026 Multi-Node Determinism & Migration ✅

**Status**: Done (Milestones 1-4, 7-8)

### Tasks
- [x] **Safe Teardown**.
- [x] **Session Pool**.
- [x] **Hardware Jitter Profile Injection (Chaos Engineering)**.
- [x] **Unix Sockets**.
- [x] **Coordinator Barrier**.
- [x] **Wireless Topology**.
- [x] **Deterministic Seeding**.
- [x] **Unified PCAP**.

---

## Architectural Hardening — Quantum 2026 ✅

### Tasks
- [x] **GLOBAL_CLOCK TOCTOU**.
- [x] **RAII BQL**.
- [x] **Atomic State Machine**.
- [x] **Sequence Numbers**.
- [x] **Admission Control**.
- [x] **Overshoot Compensation**.

---

## Milestone 20.5 — SPI Bus & Peripherals ✅

### Tasks
- [x] **20.5.1**: SSI/SPI bindings.
- [x] **20.5.2**: PL022 verification.
- [x] **20.5.3**: spi bridge.
- [x] **20.5.4**: spi_echo.elf verification.

---

## Milestone 29 — Peripheral Time Fidelity & Backpressure ✅

### Tasks
- [x] **29.1**: QEMUTimer modeling.
- [x] **29.2**: UART Backpressure.
- [x] **29.3**: RX Propagation.
- [x] **29.4**: Radio Delays.
- [x] **29.5**: Lifecycle Assertions.

---

## Miscellaneous Hardening & Infrastructure ✅

### Tasks
- [x] **Automated Flight Recorder (Record & Replay)**.
- [x] **Watchdog**.
- [x] **Multiplier**.
- [x] **P10-Part 2.1**: Zenoh Liveliness.
- [x] **R19**: Fatal Audit.
- [x] **30.6**: remote-port Rust.
- [x] **Backlog Admission Control**.
- [x] **Std cleanup**.
- [x] **Sync Protocol**.
- [x] **Session Isolation**.
- [x] **Quantum Alignment**.
- [x] **Test Infrastructure Consolidation**.
- [x] **Watchdog**.
- [x] **Transport Agnostic organizes**.
- [x] **Fixed Miri tests**
