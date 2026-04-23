# virtmcu Completed Phases

This file serves as a historical record of completed phases and tasks in the virtmcu project.

---

## Architectural Hardening — ASan & UAF Prevention ✅

**Status**: Done

### Tasks
- [x] **Milestone 1**: `SafeSubscriber` RAII wrapper implemented in `virtmcu-zenoh` to automatically manage BQL and teardown races.
- [x] **Milestone 2**: `QomTimer` RAII wrapper implemented in `virtmcu-qom` for automated timer destruction.
- [x] **Milestone 3**: All Rust peripherals (`zenoh-chardev`, `s32k144-lpuart`, `zenoh-netdev`, `zenoh-canfd`, `zenoh-flexray`, `zenoh-802154`, `zenoh-ui`) refactored to use the safe wrappers.

---

## Phase 0 — Repository Setup ✅

**Status**: Done

### Tasks
- [x] Directory scaffold: `hw/`, `tools/repl2qemu/`, `tools/testing/`, `scripts/`, `docs/`
- [x] `CLAUDE.md` — AI agent context file (architecture decisions, constraints, local paths)
- [x] `PLAN.md` — initial implementation plan
- [x] `README.md` — human-readable overview
- [x] `docs/ARCHITECTURE.md` — consolidated QEMU vs Renode analysis
- [x] `.gitignore` updated for `modules/`, `build/`, `*.so`, `*.dtb`, `.venv/`

---

## Phase 1 — QEMU Build with arm-generic-fdt ✅

**Goal**: A working QEMU binary on Linux with `--enable-modules` and the arm-generic-fdt machine type.

### Tasks
- [x] **1.1** Write `scripts/setup-qemu.sh`
- [x] **1.2** Write a minimal `test/phase1/minimal.dts`
- [x] **1.3** Write `scripts/run.sh` skeleton
- [x] **1.4** Smoke-test: boot the minimal DTB
- [x] **1.5** Write tutorial lesson 1: Dynamic Machines, Device Trees, and Bare-Metal Debugging.

---

## Phase 2 — Dynamic QOM Plugin Infrastructure ✅

**Goal**: Compile a minimal out-of-tree QOM peripheral as a `.so` and load it into QEMU.

### Tasks
- [x] **2.1** Write `hw/rust/rust-dummy` (Migrated from C)
- [x] **2.2** Update QEMU module build configuration
- [x] **2.3** Verify the native module loading
- [x] **2.4** Add a Rust template
- [x] **2.5** Write tutorial lesson 2: Creating and Loading Dynamic QOM Plugins.

---

## Phase 3 — repl2qemu Parser ✅

**Goal**: Parse a Renode `.repl` file and produce a valid `.dtb`.

### Tasks
- [x] **3.1** Obtain reference `.repl` files
- [x] **3.2** Write `tools/repl2qemu/parser.py`
- [x] **3.3** Write `tools/repl2qemu/fdt_emitter.py`
- [x] **3.4** Write `tools/repl2qemu/cli_generator.py`
- [x] **3.5** Write `tools/repl2qemu/__main__.py`
- [x] **3.6** Unit tests in `tests/repl2qemu/test_parser.py`
- [x] **3.7** Write tutorial lesson 3: Parsing .repl files.
- [x] **3.8** Write integration test `test/phase3/smoke_test.sh`.

---

## Phase 5 — Co-Simulation Bridge ✅

**Goal**: Enable SystemC peripheral models to connect to QEMU via MMIO socket bridge.

### Tasks
- [x] **5.1** Implement `hw/rust/mmio-socket-bridge` (Migrated from C) and `tools/systemc_adapter/`
- [x] **5.4** Document Path A vs B vs C decision guide.
- [x] **5.5** Write tutorial lesson 5: Hardware Co-simulation and SystemC bridges.
- [x] **5.6** mmio-socket-bridge: add per-operation timeout and disconnection handling.
- [x] **5.7** High-Frequency MMIO Stress Test.
- [x] **5.8** Bridge Resilience & Reconnection Hardening.

---

## Phase 8 — Interactive and Multi-Node Serial (UART) ✅

**Goal**: Extend deterministic I/O to serial ports and provide an interactive experience.

### Tasks
- [x] **8.1** Interactive Echo Firmware.
- [x] **8.2** Tutorial Lesson 8.
- [x] **8.3** Deterministic Zenoh Chardev (Migrated to Rust: `hw/rust/zenoh-chardev`).
- [x] **8.4** Multi-Node UART Test.
- [x] **8.5** Fix `libc::malloc` without null-check in `zenoh-chardev` and `zenoh-802154`.
- [x] **8.6** High-Baud UART Stress Test.

---

## Phase 9 — Advanced Co-Simulation: Shared Media (SystemC) ✅

**Goal**: Model complex shared physical mediums (like CAN or SPI) in SystemC with asynchronous interrupt support.

### Tasks
- [x] **9.1** Asynchronous IRQ Protocol.
- [x] **9.2** Multi-threaded SystemC Adapter.
- [x] **9.3** Educational CAN Model.
- [x] **9.4** Tutorial Lesson 9.

---

## Phase 10 — Telemetry Injection & Physics Alignment (SAL/AAL) ✅

**Goal**: Implement standardized sensor/actuator abstraction layers.

### Tasks
- [x] **10.1** SAL/AAL Abstraction Interfaces.
- [x] **10.2** RESD Ingestion Engine.
- [x] **10.3** Zero-Copy MuJoCo Bridge.
- [x] **10.4** OpenUSD Metadata Tool.
- [x] **10.5** Tutorial Lesson 10.
- [x] **10.6** Native Zenoh Actuator Support.

---

## Phase 11 — RISC-V Expansion & Framework Maturation ✅

**Goal**: Expand architecture support to RISC-V and establish Path B co-simulation.

### Tasks
- [x] **11.1** RISC-V Machine Generation.
- [x] **11.2** Virtual-Time-Aware Timeouts.
- [x] **11.3** Remote Port Co-Simulation (Path B).
- [x] **11.4** FirmwareStudio Upstream Migration.

---

## Phase 12 — Advanced Observability & Interactive APIs ✅

**Goal**: Implement deterministic telemetry tracing and dynamic network topology API.

### Tasks
- [x] **12.1** Deterministic Telemetry Tracing.
- [x] **12.2** Dynamic Network Topology API.
- [x] **12.3** Standardized UI Topics.
- [x] **12.4** Tutorial Lesson 12.
- [x] **12.5** Concurrency inside `irq_slots`.
- [x] **12.6** Struct Protocol Rigidity (FlatBuffers).
- [x] **12.7** Safe QOM Path Resolution for IRQs.
- [x] **12.8** Telemetry Throughput Benchmark.

---

## Phase 13 — AI Debugging & MCP Interface ✅

**Goal**: Provide an MCP server for semantic interaction with the simulation.

### Tasks
- [x] **13.1** MCP Lifecycle Tools.
- [x] **13.2** Semantic Debugging API.
- [x] **13.3** Zenoh-MCP Bridge.
- [x] **13.4** Tutorial Lesson 13.

---

## Phase 14 — Wireless & IoT RF Simulation (BLE, Thread, WiFi) ✅

**Goal**: Deterministic simulation of wireless transceivers.

### Tasks
- [x] **14.1** HCI over Zenoh (BLE).
- [x] **14.2** 802.15.4 / Thread MAC.
- [x] **14.3** RF Propagation Models.
- [x] **14.4** Tutorial Lesson 14.
- [x] **14.5** True 802.15.4 MAC State Machine (Rust).
- [x] **14.6** O(N²) RF Coordinator Scaling fix.
- [x] **14.7** Dynamic Topology updates from physics.
- [x] **14.8** RF Header Schema Rigidity fix (FlatBuffers).
- [x] **14.9** Isotropic RF Assumptions improvements.

---

## Phase 15 — Distribution & Packaging ✅

**Goal**: Distribute `virtmcu` as an easily installable suite.

### Tasks
- [x] **15.1** Python Tools PyPI Package.
- [x] **15.2** Binary Releases via GitHub Actions.
- [x] **15.3** Tutorial Lesson 15.

---

## Phase 16 — Performance & Determinism CI ✅

**Goal**: Establish rigorous performance regression testing.

### Tasks
- [x] **16.1** IPS Benchmarking.
- [x] **16.2** Latency Tracking.
- [x] **16.3** Tutorial Lesson 16.
- [x] **16.4** Jitter Injection Determinism Test.
- [x] **16.5** Automated Performance Trend Tracking.

---

## Phase 17 — Security & Hardening (Fuzzing) ✅

**Goal**: Protect the simulation boundary via fuzzing.

### Tasks
- [x] **17.1** Network Boundary Fuzzing.
- [x] **17.2** Parser Fuzzing.
- [x] **17.3** Tutorial Lesson 17.

---

## Phase 18 — Native Rust Zenoh Migration (Oxidization) ✅

**Goal**: Eliminate `zenoh-c` FFI layer by rewriting core plugins in native Rust.

### Tasks
- [x] **18.1** Enable QEMU Rust Support.
- [x] **18.2** Native Zenoh-Clock (Rust).
- [x] **18.3** Native Zenoh-Netdev (Rust).
- [x] **18.4** Native Zenoh-Telemetry (Rust).
- [x] **18.5** Native Zenoh-Chardev, Actuator, 802154, UI (Rust).
- [x] **18.6** Verification & CI Integration.
- [x] **18.7** Fix BQL in `zenoh-clock` (Rust).
- [x] **18.8** Fix `zenoh-telemetry` wrong return type.
- [x] **18.9** Adopt `virtmcu-qom` in `zenoh-clock`.
- [x] **18.10** Adopt `virtmcu-qom` in `zenoh-netdev`.
- [x] **18.11** Align Cargo.toml workspace fields.
- [x] **18.12** Zenoh session helper.
- [x] **18.13** Rust FFI Safety & Memory Audit.
- [x] **18.14** Lock-Free Priority Queue Evaluation.

---

## Phase 19 — Native Rust QOM API Migration ✅

**Goal**: Eliminate all C shim files in `hw/zenoh/`, leaving Zenoh device logic fully in Rust.

### Tasks
- [x] **19.1** Expand `virtmcu-qom` for QOM type registration.
- [x] **19.2** Eliminate C shims — non-netdev devices.
- [x] **19.3** Eliminate C shim — `zenoh-netdev` (Migrated to full Rust).
- [x] **19.4** Delete `virtmcu-rust-ffi.c/h` (Done).
- [x] **19.5** Memory Layout Verification Suite.
- [x] **19.6** Refactor `virtmcu-qom` bindgen lint suppression.
- [x] **19.7** Phase 19 Critique and Stabilization.
- [x] **19.8** Phase 19 Jitter Fix.

---

## Phase 20 — Shared Rust API Crate (`virtmcu-api`) ✅

**Goal**: Provide a stable, public `rlib` for serialization schemas and Zenoh headers.

### Tasks
- [x] **20.1** Create `virtmcu-api` crate.
- [x] **20.2** Refactor Internal Plugins to use `virtmcu-api`.

---

## Phase 25 — Local Interconnect Network (LIN) ✅

**Goal**: Emulate LIN buses for automotive body control.

### Tasks
- [x] **25.1** LIN Controller Emulation.
- [x] **25.2** Master/Slave Synchronization.
- [x] **25.3** Firmware Sourcing.
- [x] **25.4** Multi-Node LIN Verification.

---

## Phase 31 — Advanced CI & Build Pipeline Optimization ✅

**Goal**: Optimize developer feedback loop and eliminate build bottlenecks.

### Tasks
- [x] **31.1** Eliminate Cargo + Ninja Lock Contention.
- [x] **31.2** Selective CI Execution.
- [x] **31.3** Python Test Parallelization (`pytest-xdist`).
- [x] **31.4** Deep C Static Analysis (`cppcheck`).
- [x] **31.5** LLVM Linker (`lld`) for QEMU & Rust.
- [x] **31.6** Universal Typo Prevention (`codespell`).
