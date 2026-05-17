# virtmcu Active Implementation Plan

**Goal**: VirtMCU turns QEMU into a Binary-Compatible Deterministic Simulation framework for Distributed Systems. It supports dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation. The software MUST be at the highest Enterprise Quality following the SOTA of software development.

**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

**Language rule**: Rust everywhere except simple CI glue. No Bash for orchestration. `DataTransport` is the only peripheral/coordinator API.

---

## [P0] Enterprise SOTA: Type-Safe Framework Boundaries (RFC-0041) — DONE (P0.1–P0.5)

P0.1–P0.5 are complete: `BqlContext` token, `BqlGuarded` upgrade, `ClosureTimer`, `dynamic_cast_qom`, and the reference-peripheral pilot migration are all implemented and passing `make test-lint`. See git history for the full checklist.

### P0.6 — Full Test Suite Green for Reference Peripheral

**Goal**: All three test tiers pass for the migrated reference peripheral. Empirical proof that RFC-0041 is correct and complete before mass migration begins.

- [ ] P0.6a: **Unit tests** (`make test-unit`): All `virtmcu-qom` unit tests pass.
- [ ] P0.6b: **Integration tests**: `test_reference_ping_pong_unix` and `test_reference_ping_pong_zenoh` pass.
- [ ] P0.6c: **E2E parity test**: `test_reference_ping_pong_transport_parity` passes (both transports produce identical output).
- [ ] P0.6d: **Sanitizer gate** (ASAN + Miri on `virtmcu-qom`): No UAF, no leak, no drop-order violations, no data races.
- [ ] P0.6e: **Compile-fail tests**: `BqlContext` cannot be sent across a thread boundary; `qemu_clock_get_ns_safe` without `&BqlContext` fails to compile.

**Gate**: `make ci-full` exits 0. No regressions in any existing test.

---

## [P1] Mass Migration + Hardening

*Blocked on P0.6 (all reference-peripheral tests green).*

### Task 1 — Peripheral Coverage Audit
**Goal**: Before mass migration, enumerate every peripheral and classify its test coverage. No peripheral migrates without a pre-existing test.
- [x] 1.1: Enumerate all `DrainToken`, `deref_qom_ptr`, `opaque_to_state`, and standalone `extern "C"` timer callbacks across `hw/rust/buses`, `bridges`, `physics`, `mcu`, `observability`.
- [x] 1.2: For each peripheral with no test: write a minimal smoke test before migration.

**Gate**: Every peripheral has at least one test. Coverage matrix documented.

### Task 2 — Mass Migration to RFC-0041 APIs
**Goal**: Apply the same mechanical migration performed on reference-peripheral to all peripherals.
- [ ] 2.1: Apply RFC-0041 migration checklist to each peripheral (`DrainToken` → `BqlContext`, `BqlGuarded` ctx, `ClosureTimer`, `dynamic_cast_qom`).
- [ ] 2.2: Each migrated peripheral must pass its tests before the next peripheral is started.
- [ ] 2.3: Remove `deref_qom_ptr` and `opaque_to_state` from `timer.rs` once all callers are migrated.

**Gate**: `grep -r "DrainToken\|deref_qom_ptr\|opaque_to_state" hw/rust/` returns zero matches.

### Task 3 — Sanitizer Gates for All Peripherals
- [ ] 3.1: Every migrated peripheral has a teardown integration test (teardown during blocked MMIO, ASAN/TSAN clean).
- [ ] 3.2: Add ASAN/TSAN run to `make ci-full` for the peripheral crates.

### Task 4 — Remaining Core Hardening
- [ ] 4.1: Migrate `GLOBAL_CLOCK` and `GLOBAL_TELEMETRY` to `VIRTMCU_EXPORT`-based registration. Eliminates the DSO Boundary Isolation Trap for global singletons.
- [ ] 4.2: Implement configurable `VIRTMCU_ZENOH_CONNECT_TIMEOUT_MS` to prevent router discovery from blocking the QEMU main thread.
- [ ] 4.3: Fully transition all core I/O to FlatBuffers (`vproto`) accessor patterns, removing manual `read_unaligned` and raw casts.

### Task 5 — Deep Testing Overhaul
- [ ] 5.1: Firmware coverage gate: CI `drcov` step fails build if coverage drops below 80%.
- [ ] 5.2: Migrate fragile Bash test orchestration scripts to Rust test runner.

### Task 6 — Execution Pacing
- [ ] 6.1: Host timeout scale documentation.
- [ ] 6.2: Coordinator `--pacing <float>` flag.
- [ ] 6.3: FTRT proof test.

---

## [P1.5] Topic-Free Coordinator Protocol (RFC-0042)

*Blocked on P1 Task 2 (all peripherals migrated to RFC-0041). Unblocks P2 by preventing topic-string debt from accumulating in new peripherals.*

**Thesis**: The coordinator routes data frames by substring-matching topic strings (`topic.contains("chardev")`). Every new peripheral or protocol rename touches five files and the silent `else { Protocol::Ethernet }` fallback violates RFC-0022. RFC-0042 replaces this with a **hub-and-spoke model**: each node has one UDS socket (already the case); each link in the topology YAML gets one `u32 link_id`; the coordinator routes `(link_id, payload)` to all other participants. Connections grow O(N), link IDs grow O(M), multi-cast requires zero special cases.

### Stage 1 — Foundation (Flag-Day)
- [ ] S1.1: Add `name:` field to all topology link declarations; `yaml2qemu` hard-errors on missing or duplicate names; provide `yaml2qemu migrate-link-names` to auto-generate names for existing files.
- [ ] S1.2: `yaml2qemu` injects `link-name` QOM property into each participating peripheral; hard lint error if `topic:` property is present; emits build-time deprecation warning when Zenoh transport is selected.
- [ ] S1.3: Add `LinkRole` enum + `LinkRegistration` + `LinkAck` FlatBuffer tables to `hw/rust/common/virtmcu-wire/src/core.fbs`; regenerate via `cargo xtask flatc`; bump `UDS_PROTO_VERSION`.
- [ ] S1.4: Coordinator startup: build `link_ids: HashMap<link_name, link_id>` and `rx_map: HashMap<link_id, Vec<node_id>>` (all participants; sender excluded at delivery). O(M) entries regardless of node count or broadcast topology.
- [ ] S1.5: Coordinator: handle `sim/coord/link/register`; validate `(node_id, link_name, protocol)` against topology; respond with `LinkAck { link_id }` — same `link_id` for all participants of the same link; panic with named diagnostics on mismatch.
- [ ] S1.6: Pre-flight barrier: block `sim/coord/start` until all `(node_id, link_name)` pairs from topology have registered; 30 s timeout; panic names every missing pair.
- [ ] S1.7: Coordinator dual-mode: accept both legacy `sim/chardev/{n}/tx` topics AND new `sim/ch/{link_id}` topics; route `sim/ch/*` by `rx_map` lookup and hub fan-out to all other participants' sockets; panic on unknown link_id.
- [ ] S1.8: `DataTransport` trait: add `register_link() -> u32` (returns `link_id`) and `reserve_link(link_id, …)`; mark `reserve(topic, …)` `#[deprecated]`.

**Gate**: `make test-check` green. Coordinator accepts both legacy and link-ID frames. Multi-cast (CAN, RF) requires zero coordinator code changes vs. point-to-point.

### Stage 2 — Peripheral Migration (Mechanical, Protocol-by-Protocol)
- [ ] S2.1: Migrate `reference-peripheral` first: replace `topic` with `link_name`; call `register_link()` in `realize()`; use `reserve_link(link_id, …)` in write; use `VtimeIngress::new_for_link(link_id, …)` for RX.
- [ ] S2.2: Add `VtimeIngress::new_for_link(link_id, …)` API; deprecate `new(topic, …)`.
- [ ] S2.3: Migrate remaining peripherals one at a time: CAN-FD, UART, Ethernet, FlexRay, Sensor, SPI, RF.
- [ ] S2.4: Extend `banned_patterns` lint: prohibit new `topic:` QOM property declarations in `hw/rust/`.

**Gate**: All integration tests green. `grep -r '"topic"' hw/rust/` returns zero matches in production code.

### Stage 3 — Delete Legacy Path
- [ ] S3.1: Remove all `topic.contains(…)` substring routing, wildcard constants, topic template functions, `base_topic` field on `CoordMessage`.
- [ ] S3.2: Remove `DataTransport::reserve(topic, …)` and `VtimeIngress::new(topic, …)` — **only after** the Zenoh transport is also migrated (coordinated with Zenoh follow-on RFC; do not remove while Zenoh still uses topic strings).
- [ ] S3.3: Remove `topic` QOM property from all peripheral structs and world YAMLs.
- [ ] S3.4: `yaml2qemu` hard-errors on `topic:` property (upgrade from Stage 1 lint-warn).

**Gate**: `make ci-full` green. `grep -r "chardev_tx\|chardev_rx\|CHARDEV_TX_WILDCARD\|base_topic" .` returns zero matches.

---

## [P2] Hardware Expansion & Peripherals

### Task 20 — CAN-FD (Bosch M_CAN)
Frame delivery routes through coordinator (RFC-0024). No raw Zenoh pub/sub.
- [ ] 20.1: Implement missing Bosch M_CAN register logic.
- [ ] 20.2: Enable CAN-FD frame delivery through `DeterministicCoordinator` via `DataTransport`.
- [ ] 20.3: Pass vendor SDK loopback/echo tests.

### Task 21 — FlexRay (Automotive)
- [ ] 21.1: Add FlexRay IRQ lines.
- [ ] 21.2: Implement Bosch E-Ray Message RAM Partitioning.
- [ ] 21.3: Fix SystemC build regression (CMake 4.3.1 compatibility).

### Task 22 — WiFi (802.11)
Radio propagation lives in a physics gateway federate, not the peripheral.
- [ ] 22.1: Harden `arm-generic-fdt` Bus Assignment.
- [ ] 22.2: Formalize `wifi` Rust QOM Proxy.
- [ ] 22.3: Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### Task 23 — Thread Protocol
*Depends on Task 22.*
- [ ] 23.1: Deterministic Multi-Node UART Bus Bridge.
- [ ] 23.2: SPI 802.15.4 Co-Processor (e.g., AT86RF233).

### Task 24 — Vendor Firmware Validation (Binary Fidelity)
- [ ] 24.1: CAN-FD (Bosch M_CAN).
- [ ] 24.2: Ethernet (MAC).

### Task 25 — Connectivity Expansion
- [ ] 25.1: Bluetooth (nRF52840 RADIO emulation).
- [ ] 25.2: Automotive Ethernet (100BASE-T1).

---

## [P3] Strategic Evolution

### Task 30 — Enterprise SSoT Generation
- [ ] 30.1: Phase 1: DRYing the World (YAML composition).
- [ ] 30.2: Phase 2: IDL Bridge (TypeSpec).
- [ ] 30.3: Phase 3: USD Migration (OpenUSD).
- [ ] 30.4: `virtmcu-cli platform generate` (YAML → DTB).

### Task 31 — Real-Time Visualization
- [ ] 31.1: Simulation Gateway (Rust/Axum backend).
- [ ] 31.2: Transport Agnostic Observer.
- [ ] 31.3: Frontend (React Flow topology graph).
- [ ] 31.4: AI Integration (MCP).

### Task 32 — Enterprise Sensor Data Replay & Telemetry
- [ ] 32.1: Add `mcap` to workspace.
- [ ] 32.2: Build `virtmcu-replay` as a deterministic co-simulation node.
- [ ] 32.3: `mdf2mcap` Converter.
- [ ] 32.4: Deprecate RESD.

### Task 33 — External Input Boundaries & Interactive Mode
*Depends on coordinator UDS server (complete).*
- [ ] 33.1: Add `boundary` field to topology YAML. Coordinator panics at startup if missing or unknown.
- [ ] 33.2: Coordinator external input endpoint: UDS socket per `boundary: interactive` link.
- [ ] 33.3: vtime stamping of arriving bytes.
- [ ] 33.4: Input log recording to MCAP.
- [ ] 33.5: Replay mode via `virtmcu-replay` node.
- [ ] 33.6: Interactive mode rate control (`--pacing 1.0`).

*Gate*: Interactive session MCAP log replays to bit-identical QEMU output.

---

## Ongoing Risks (Watch List)

| ID | Risk | Status / Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | QEMU version pinned; all patches via `virtmcu-cli setup patch-qemu`. Track upstream on each QEMU bump. |
| R7 | `icount` performance | Use `slaved-icount` only when sub-quantum timing precision is required; `slaved-suspend` is default. |
| R18 | No firmware coverage gate | Binary fidelity is the #1 invariant but there is no `drcov` CI gate yet. Tracked as Task 5.1. |
