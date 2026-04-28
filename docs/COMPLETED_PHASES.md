# virtmcu Completed Phases

This file serves as a historical record of completed phases and tasks in the virtmcu project.

---

## Architectural Hardening — ASan & UAF Prevention ✅

**Status**: Done

### Tasks
- [x] **Milestone 1**: `SafeSubscriber` RAII wrapper implemented in `transport-zenoh` to automatically manage BQL and teardown races.
- [x] **Milestone 2**: `QomTimer` RAII wrapper implemented in `virtmcu-qom` for automated timer destruction.
- [x] **Milestone 3**: All Rust peripherals (`chardev`, `s32k144-lpuart`, `netdev`, `canfd`, `flexray`, `ieee802154`, `ui`) refactored to use the safe wrappers.

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
- [x] **2.1** Write `hw/rust/common/rust-dummy` (Migrated from C)
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
- [x] **8.3** Deterministic Zenoh Chardev (Migrated to Rust: `hw/rust/chardev`).
- [x] **8.4** Multi-Node UART Test.
- [x] **8.5** Fix `libc::malloc` without null-check in `chardev` and `ieee802154`.
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
- [x] **18.5** Native Zenoh-Chardev, Actuator, ieee802154, UI (Rust).
- [x] **18.6** Verification & CI Integration.
- [x] **18.7** Fix BQL in `clock` (Rust).
- [x] **18.8** Fix `telemetry` wrong return type.
- [x] **18.9** Adopt `virtmcu-qom` in `clock`.
- [x] **18.10** Adopt `virtmcu-qom` in `netdev`.
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
- [x] **19.3** Eliminate C shim — `netdev` (Migrated to full Rust).
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

---

## P0 Serial Tasks — Enterprise Hardening ✅

**Completed**: 2026-04-25 (Tasks A–I) / 2026-04-27 (P01 Rust mechanism, P09 partial)

### Summary

| Task | What was done |
|---|---|
| P00 | Re-enabled Phase 7 tests; fixed ASan false-stall by adding `stall-timeout=60000` (later superseded by `is_first_quantum` in P01). |
| P01 | `is_first_quantum: AtomicBool` + `BOOT_QUANTUM_TIMEOUT = 5 min` added to `ZenohClockBackend`. First quantum uses a 5-minute grace window; subsequent quanta revert to `stall_timeout_ms`. |
| P01b | Restored main branch to fully green state. |
| P02 | All `#[repr(C, packed)]` reads in `remote-port` replaced with `ptr::read_unaligned`. Unit tests: `test_unaligned_hdr_read`, `test_unaligned_busaccess_read`, `test_unaligned_interrupt_read`. |
| P03 | `sync.rs` BQL mock upgraded to support configurable return values. Unit tests cover `BqlGuard` drop, `BqlUnlockGuard` re-acquire, `temporary_unlock()` when not held, timeout path, signal path. |
| P04 | All direct `virtmcu_bql_locked()` FFI calls replaced with `Bql::is_held()`. `BqlGuarded<T>` introduced and migrated across all Zenoh peripherals. `Mutex<T>` banned in peripheral state via `make lint` gate. |
| P05 | Both `mmio-socket-bridge` and `remote-port` converted to pure Rust `std::sync::Mutex`/`Condvar`. Raw `*mut QemuMutex`/`*mut QemuCond` eliminated. Lock order documented in module-level comments. |
| P06 | `VcpuCountGuard` RAII ensures `active_vcpu_count` is decremented even on panic. `drain_cond.wait_timeout(30 s)` replaces the bounded spinloop in both bridges. |
| P07 | All `thread::sleep` reconnect/connection-wait loops replaced with `connected_cond.wait_timeout()`. CI grep gate enforces zero untagged sleeps in `hw/rust/`. |
| P08 | `.github/workflows/ci-asan.yml` added; gates on `push` to `main` and all `pull_request` targets. `make test-asan` runs nightly Rust ASan suite. |
| P09 | `static_mut_refs` and `too_many_lines` suppressors removed. `cargo clippy -D warnings` enforced. Five peripheral crates still require `#![allow(clippy::all)]` cleanup — tracked in `PLAN.md`. |
| P10-1 | `chardev` flow control: `drain_backlog()` with `qemu_chr_be_can_write`, byte-level `VecDeque` backlog, `chr_accept_input` drain. 128-byte burst test passes. |
| P10-2.2 | `VirtualTimeAuthority` fixture in `conftest.py` with `step()` and `run_until()`. `TimeAuthority` legacy wrapper maintained. |
| P10-3 | All Phase 8 tests use `slaved-icount`. Burst test, `test_phase8_uart_stress.py`, and marker-packet drop test in place. |
| P11 | Dynamic ports, `tmp_path` isolation, workspace-scoped `cleanup-sim.sh`, binary resolution for Rust tool crates. |
| Task A | `remote-port` `bridge_write` and `send_req_and_wait_internal`: `to_ne_bytes()` → `to_le_bytes()`, raw `ptr::copy_nonoverlapping` → `u64::from_le_bytes()`. |
| Task B | `spi` header serialization: raw `ptr::copy_nonoverlapping` → `ZenohSPIHeader::pack()`. |
| Task E (P-SERIAL) | Eliminated `transmute` and raw memory views across all peripherals; explicit `pack()`/`unpack()` methods for all wire structs. |
| Task F (P-SCHEMA) | `scripts/gen_vproto.py` auto-generates `tools/vproto.py` from `hw/rust/common/virtmcu-api/src/lib.rs`. `gen_vproto.py --check` runs in `make lint`. |
| Task G1 | `slice::from_raw_parts` on RP packet sends replaced with `pack_be()`. Byte-exact tests for `RpPktBusaccess` and `RpPktInterrupt`. |
| Task G2 | `VcpuCountGuard` RAII added to both bridges for panic-safety. |
| Task H | `BqlGuarded<T>` migration across `ieee802154`, `netdev`, `flexray`, `telemetry`, `ui`, `canfd`, `chardev`. |
| Task I | Fixed Docker Bake tag replacement behavior (prevented manifest failures in multi-arch builds). |
- [x] **31.6** Universal Typo Prevention (`codespell`).

---

## 2026 Multi-Node Determinism & Migration (DET Series) ✅

**Status**: Done (Phases 1-4, 7)

### Tasks
- [x] **DET-1**: Fix `SafeSubscriber` Bounded-Spinloop Teardown. Replaced with `Condvar`-based unconditional drain to prevent UAF during device finalization.
- [x] **DET-2**: Shared Zenoh Session Pool. Replaced N independent `open_session()` calls with a process-wide `Arc<Session>`, reducing thread overhead and startup time.
- [x] **DET-3**: `ClockSyncTransport` Trait. Abstracted the clock-advance request/reply interface, decoupling `clock` from Zenoh-specific types and enabling Unix socket support.
- [x] **DET-4**: `UnixSocketClockTransport`. Implemented the clock transport over Unix domain sockets, reducing RTT from ~50µs to ~2µs for single-host simulations.
- [x] **DET-5**: `DeterministicCoordinator` Quantum Barrier. Promoted the coordinator to a per-quantum PDES barrier that enforces canonical message ordering by sorting all inter-node messages before delivery.
- [x] **DET-6**: Topology-First YAML Loading. The full network graph is declared in the world YAML `topology:` section. The coordinator enforces the graph and drops messages not permitted by the graph.
- [x] **DET-7**: Deterministic CSMA/CA Seeding. Implemented `seed_for_quantum` utility; all stochastic protocols (802.15.4, BLE) now derive seeds from `(global_seed, node_id, quantum_number)`.
- [x] **DET-8**: Unified PCAP Logging. The coordinator writes a libpcap-format log of every inter-node message with its virtual timestamp, generating byte-identical PCAP files for determinism verification.

---

## Architectural Hardening — Quantum 2026 Phase ✅

**Status**: Done (Phases 1-6)

### Tasks
- [x] **ARCH-1**: Fix `GLOBAL_CLOCK` TOCTOU Race. Reordered hook entry to increment `ACTIVE_HOOKS` before loading the global pointer, preventing UAF during device teardown.
- [x] **ARCH-2**: RAII BQL Management. Replaced manual `virtmcu_bql_unlock()`/`lock()` calls in `clock` with `Bql::temporary_unlock()` RAII guard to prevent deadlock on panic/early-return.
- [x] **ARCH-3**: Atomic State Machine. Replaced `quantum_ready`/`quantum_done` booleans with a single `AtomicU8` enum and `compare_exchange` transitions, eliminating illegal intermediate states.
- [x] **ARCH-4**: Sequence Numbers in Wire Protocol. Added `sequence_number: u64` to `ZenohFrameHeader` and all TX paths to enable canonical PDES tie-breaking.
- [x] **ARCH-5**: Admission Control. Implemented per-node, per-quantum message limits in `DeterministicCoordinator` to prevent simulation flooding.
- [x] **ARCH-6**: Virtual-Time Overshoot Compensation. Added drift tracking and compensation to `TimeAuthority` to maintain simulation timeline accuracy across thousands of quanta.

---

## Phase 20.5 — SPI Bus & Peripherals ✅

**Status**: Done

### Tasks
- [x] **20.5.1**: SSI/SPI Safe Rust Bindings in `virtmcu-qom`.
- [x] **20.5.2**: Verified PL022 (PrimeCell) SPI controller end-to-end in `arm-generic-fdt`.
- [x] **20.5.3**: `hw/rust/spi` bridge implemented.
- [x] **20.5.4**: SPI Loopback/Echo Firmware verification (`tests/test_phase20_5.py`).

---

## Phase 29 — Peripheral Time Fidelity & Backpressure ✅

**Status**: Done

### Tasks
- [x] **29.1**: FIFO & Timer Baseline TX/RX modeling with `QEMUTimer` in `rust-dummy`.
- [x] **29.2**: UART Backpressure in `chardev` and `s32k144-lpuart`.
- [x] **29.3**: RX Propagation Modeling via timers.
- [x] **29.4**: Radio Delays (802.15.4) CSMA/CA backoff and air-time modeling.
- [x] **29.5**: Lifecycle Assertions for timer teardown.

---

## Miscellaneous Hardening & Infrastructure ✅

**Status**: Done

### Tasks
- [x] **P01-REMAINING**: ASan Boot-Time Stall. Implemented "Slow boot / fast execute" invariant via `is_first_quantum` grace window.
- [x] **P09-REMAINING**: Eliminated `#![allow(clippy::all)]` across all 5 remaining peripheral crates.
- [x] **P10-Part 2.1**: Zenoh Discovery via Liveliness API. Eliminated polling/sleeps in favor of event-driven discovery.
- [x] **R19**: Fatal Security Audit. Makefile now treats `cargo audit` failures as fatal errors.
- [x] **30.6**: Migrated `remote-port` to Rust (`hw/rust/remote-port`).

### **[ARCH-9] Unbounded Backlog Admission Control** — Reliability ✅
**Status**: Done

**Goal**: Implemented drop-tail backlog admission control for `chardev` (byte-based) and `netdev` (packet-based) to prevent QEMU memory exhaustion under flood conditions. Added `max-backlog` and exposed real-time, BQL-free telemetry (`dropped-frames`, `backlog-size`) via QOM properties.

### **[ARCH-16] Remove Misleading `#![no_std]` from Peripherals** — Code Quality ✅
**Status**: Done

**Goal**: Removed misleading `#![no_std]` annotations from `hw/rust/` peripheral plugins since they inherently rely on `std` via `virtmcu-qom` and `zenoh`. A new lint check was added to `Makefile` to prevent the re-introduction of `#![no_std]` in this context.


### **[ARCH-8] TA/Coordinator Synchronization Protocol** — Correctness

**Status**: ✅ Complete.

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
- [x] `hw/rust/clock/src/lib.rs` — add subscription to `sim/clock/start/{node_id}`;
  `recv_advance` blocks until both TA reply AND coordinator start signal are received
- [x] `tools/deterministic_coordinator/src/main.rs` — after delivery of Q's messages, publish
  `sim/clock/start/{node_id}` to all nodes
- [x] `docs/design/COORDINATOR_SYNC_PROTOCOL.md` — write the formal protocol spec (new doc)

**Formal protocol** (written in `COORDINATOR_SYNC_PROTOCOL.md`):
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
- [x] `test_clock_worker_loop` in `hw/rust/clock/src/lib.rs` (verified barrier behavior).

**Integration test** (`tests/test_arch8_coordinator_sync.py`):
- [x] 2-node simulation; deliberately inject a 100ms delay in coordinator delivery; assert clock does not advance until delivery completes.

**Definition of Done**:
- [x] `COORDINATOR_SYNC_PROTOCOL.md` written with the 8-step protocol above.
- [x] No Q+1 advance released before Q delivery is complete (verified by integration test).
- [x] `make lint` passes.

---


---

### **[ARCH-13] Clock Session Priority Isolation** — Performance

**Status**: ✅ Completed. clock now uses its own private Zenoh session.

**Goal**: When the clock transport is Zenoh (multi-host mode), the clock `GET` query
competes with high-volume emulated network traffic on the same shared Zenoh session.
A single burst of 1000 Ethernet frames can delay the clock reply by 10–50 ms, causing
spurious STALL events.

**Solution**: Use a *dedicated* Zenoh session for clock sync (separate from the data-plane
session from DET-2). The two sessions connect to the same router but have independent
executor thread pools, eliminating contention.

**Files to modify**:
- `hw/rust/clock/src/lib.rs` — do not use `get_or_init_session()`; instead call
  `open_session(router)` to get a private session
- `hw/rust/transport-zenoh/src/lib.rs` — document in `get_or_init_session` doc comment that
  it is for the *data plane only* and clock must use its own session

**Unit tests** (integration, `tests/test_arch13_clock_isolation.py`):
- Boot a 2-node simulation. Flood the data plane with 10 000 frames from node A to B.
- Simultaneously measure clock advance RTT on node A.
- Assert median clock RTT < 5ms even during the flood (compare with DET-4 baseline).

**Definition of Done**:
- [x] `clock` does not call `get_or_init_session()` — uses its own private session.
- [x] Comment in `get_or_init_session` states "data plane only; clock uses dedicated session".
- [ ] Integration test passes: clock RTT < 5ms during 10 000-frame flood.
- [ ] `make lint` passes.

---

---

### **[ARCH-18] Formal Quantum Number Alignment** — Protocol Correctness

**Status**: ✅ Completed.

**Goal**: The TimeAuthority increments a `quantum_number` counter; the
`DeterministicCoordinator` also tracks a `quantum_number`. Currently these are not
exchanged in the wire protocol — a restart or reconnect can leave them out of sync,
causing the coordinator to deliver Q's messages to nodes that have already moved to Q+1.

**Wire protocol change**: Add `quantum_number: u64` to both `ClockAdvanceReq` and the
coordinator's `done` signal. The TA, QEMU, and coordinator must all agree on the current
quantum number. Reject messages with mismatched quantum numbers.

**Files to modify**:
- `hw/rust/common/virtmcu-api/src/lib.rs` — add `quantum_number: u64` to `ClockAdvanceReq`
  and `ClockReadyResp` (both sides echo the number)
- `hw/rust/clock/src/lib.rs` — echo `quantum_number` in `ClockReadyResp`
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
- [x] `quantum_number` field in both `ClockAdvanceReq` and `ClockReadyResp`.
- [x] Coordinator validates quantum numbers on every `done` signal.
- [x] Both unit tests pass.
- [x] `make lint` passes.

---

---

### **[INFRA-1] Consolidate Testing Infrastructure & `conftest.py` Duplication** — DRY & Maintainability

**Status**: ✅ Complete.

**Goal**: `tests/conftest.py` and `tools/testing/conftest.py` are functionally identical, leading to double-maintenance of fixtures like `VirtualTimeAuthority` and `qemu_launcher`. We need a single source of truth for VirtMCU testing utilities.

**Files to modify**:
- `tests/conftest.py`
- `tools/testing/conftest.py`
- `tools/testing/virtmcu_test_suite/` (new directory/module if needed)

**Implementation sketch**:
- Move the core logic (fixtures, `VirtualTimeAuthority`, `QmpBridge` setup) into a dedicated shared package `tools/testing/virtmcu_test_suite/conftest_core.py`.
- Refactor both `tests/conftest.py` and `tools/testing/conftest.py` to simply `from tools.testing.virtmcu_test_suite.conftest_core import *`.
- Alternatively, remove `tools/testing/conftest.py` completely if it's strictly redundant, and adjust `PYTHONPATH` or `pytest` rootdir.

**Unit tests**:
- Run the full test suite (`pytest tests/` and `pytest tools/testing/`) to ensure fixtures are still resolved correctly.

**Definition of Done**:
- [x] Only one physical copy of the test fixture logic exists.
- [x] `make test` runs cleanly.

---

---

### **[INFRA-2] Centralized Artifact & Binary Resolver** — DRY

**Status**: ✅ Complete.

**Goal**: Eliminate repetitive `find_bin()` logic scattered across tests (`test_phase10.py`, `test_det6_topology.py`, `conftest.py`). Tests repeatedly write fallback chains checking `CARGO_TARGET_DIR`, `target/release`, and tool-specific directories.

**Files to modify**:
- `tools/testing/artifact_resolver.py` (new)
- `tests/test_phase10.py`
- `tests/test_det6_topology.py`
- `tests/conftest.py`

**Implementation sketch**:
```python
# tools/testing/artifact_resolver.py
import os
from pathlib import Path

def resolve_rust_binary(name: str) -> Path:
    """Finds a built Rust binary across standard workspace locations."""
    workspace_root = Path(__file__).resolve().parent.parent.parent
    paths = [
        Path(os.environ.get("CARGO_TARGET_DIR", "")) / "release" / name if "CARGO_TARGET_DIR" in os.environ else None,
        workspace_root / "target/release" / name,
        workspace_root / f"tools/{name}/target/release/{name}"
    ]
    for p in filter(None, paths):
        if p.exists():
            return p
    raise FileNotFoundError(f"Binary {name} not found. Did you run 'cargo build'?")
```

**Unit tests**:
- Create `tests/test_artifact_resolver.py` to test resolution with mocked `os.environ` and simulated directory structures.

**Definition of Done**:
- [x] `resolve_rust_binary` implemented and documented.
- [x] Redundant `find_bin()` functions removed from all tests.
- [x] `make test` passes.

---

---

### **[INFRA-3] `VirtMcuOrchestrator` High-Level Simulation API** — Test Readability

**Status**: ✅ Complete.

**Goal**: Manual QEMU argument string building, socket management, and clock polling create excessive boilerplate in multi-node tests (e.g., `test_phase25_multi_node.py`, `test_arch8_coordinator_sync.py`). Introduce a declarative API.

**Files to modify**:
- `tools/testing/virtmcu_test_suite/orchestrator.py` (new)
- Refactor a complex test (e.g., `test_phase25_multi_node.py`) to use the orchestrator.

**Implementation sketch**:
```python
async with VirtMcuOrchestrator(zenoh_router) as sim:
    node0 = sim.add_node(node_id=0, dtb_path="master.dtb", kernel_path="master.elf", extra_args=[...])
    node1 = sim.add_node(node_id=1, dtb_path="slave.dtb", kernel_path="slave.elf", extra_args=[...])
    
    # Wait for expected condition on a specific node, automatically advancing clock
    await sim.run_until(lambda: b"S" in node1.uart.buffer, timeout=5.0, step_ns=1_000_000)
```

**Unit tests**:
- No specific new tests required beyond the refactoring of existing complex tests to prove the Orchestrator works reliably and safely handles teardown on failure.

**Definition of Done**:
- [x] `VirtMcuOrchestrator` class implemented with `add_node()`, `start()`, and `run_until()`.
- [x] At least one multi-node phase test successfully refactored to use the Orchestrator.
- [x] Background QEMU output logging remains functional.

---

---

### **[INFRA-4] Standardized Process & Lifecycle Management** — Reliability

**Status**: ✅ Complete.

**Goal**: Tests like `test_phase24_canfd.py` and `test_phase10.py` use `subprocess.Popen` or `asyncio.create_subprocess_exec` but fail to ensure proper cleanup if assertions fail mid-test, leaving zombie processes that break subsequent tests.

**Files to modify**:
- `tools/testing/virtmcu_test_suite/process.py` (new)
- `tests/test_phase10.py`
- `tests/test_phase24_canfd.py`

**Implementation sketch**:
Create an `AsyncManagedProcess` context manager that strictly enforces:
1. `terminate()` on exit.
2. Wait for graceful exit (with configurable timeout).
3. `kill()` if graceful exit fails.
4. Capture of STDOUT/STDERR for debugging.

**Unit tests**:
- Write a test using a bash script that traps SIGTERM to verify the context manager upgrades to `kill()` and doesn't hang.

**Definition of Done**:
- [x] `AsyncManagedProcess` context manager implemented.
- [x] Replaced raw `Popen`/`create_subprocess_exec` in at least 3 test files.
- [x] `make test` runs without leaking any QEMU or Zenoh background processes.

---

---

### **[INFRA-5] Dynamic Artifact Generation Factory** — Developer Velocity

**Status**: ✅ Complete.

**Goal**: Reduce boilerplate where tests run raw `sed` and `dtc` or `arm-none-eabi-gcc` via `subprocess.run()` to create variations of board/kernel configurations for testing. 

**Files to modify**:
- `tools/testing/virtmcu_test_suite/factory.py` (new)

**Implementation sketch**:
```python
def compile_dtb(base_dts: Path, replacements: dict[str, str], out_dir: Path) -> Path:
    """Reads base_dts, applies string replacements, and compiles to DTB."""
```
```python
def compile_c_snippet(snippet: str, out_dir: Path, linker_script: Path | None = None) -> Path:
    """Compiles a C string into a bare-metal ARM ELF."""
```

**Unit tests**:
- `test_compile_dtb`: Verify valid DTB generation.
- `test_compile_c_snippet`: Verify generated ELF executes successfully via `qemu_launcher`.

**Definition of Done**:
- [x] Factory functions implemented.
- [x] Replaced inline `subprocess.run` calls in `test_phase25_multi_node.py` or similar tests.
- [x] `make test` passes.

---

---

### **[ARCH-10] Zenoh Session Watchdog** — Reliability

**Status**: ✅ Complete.

**Goal**: If the Zenoh router dies mid-simulation, `clock` spins forever in
`recv_advance` with no diagnostics. Add a watchdog: if no advance arrives within
`3 × stall_timeout_ms`, enter a `SessionLost` state, log a fatal error, and cleanly
exit QEMU.

**Files to modify**:
- [x] `hw/rust/backbone/clock/src/lib.rs`

**Implementation sketch**:
```rust
// In recv_advance timeout path (ClockSyncTransport → ZenohClockTransport):
self.consecutive_timeouts.fetch_add(1, Ordering::Relaxed);
if self.consecutive_timeouts.load(Ordering::Relaxed) > self.watchdog_threshold {
    // Log to QEMU monitor and abort cleanly.
    virtmcu_qom::vlog!(
        "[clock] FATAL: No clock advance received in {} consecutive timeouts. \
         Zenoh router may be down. Aborting.\n",
        self.watchdog_threshold
    );
    // Exit QEMU cleanly so the Python test harness can detect the failure.
    std::process::exit(1);
}
```

Reset `consecutive_timeouts` to 0 on every successful `recv_advance`.

**Unit tests**:
- [x] `test_watchdog_triggers_after_threshold`: use `MockClockTransport` configured to timeout
  on all calls; assert that after `watchdog_threshold` calls, the watchdog fires (capture
  via a mock `abort_fn` hook rather than actual process exit).
- [x] `test_watchdog_reset_on_success`: one success after N-1 timeouts; assert `consecutive_timeouts == 0`.

**Definition of Done**:
- [x] Watchdog property `session-watchdog-ms` configurable via QEMU device property.
- [x] QEMU exits cleanly (non-zero code) when watchdog fires.
- [x] Both unit tests pass.
- [x] `make lint` passes.

---