# virtmcu Active Implementation Plan

**Goal**: VirtMCU turns QEMU into a Binary-Compatible Deterministic Simulation framework for
Distributed Systems — unmodified firmware ELFs run identically on real hardware and in
simulation.

**Guiding Principles**: KISS · YAGNI · Crash-Only Design (RFC-0022) · RAII + DI (RFC-0031)

**Immediate North Star**: Reference peripheral is the single gold-standard peripheral.
All three test tiers (unit → integration → e2e) must be fully green before any other
peripheral work begins.

---

## [PRE-0] Restore Basic Compilation — BLOCKED: everything else depends on this

> `cargo build --workspace` currently fails. Nothing can be tested until this is fixed.

- [ ] PRE-0.1: **Fix `deterministic_coordinator` main.rs**
  - Add `DummyVTimeProvider` struct + `VTimeProvider` impl (copy pattern from `virtmcu-cli`)
  - Fix `wire_link.destinations` → `wire_link.nodes` (field was renamed in topology.rs)
  - Fix `wire_link.link_id` → derive link_id from iterator index (WireLink has no link_id field)
- [ ] PRE-0.2: **Add `decode_frame` / `encode_frame` shims to `virtmcu-wire`**
  - 8 peripheral crates call these; they were deleted in a prior session without porting callers
  - Format: `[vtime_ns: u64 LE][seq_num: u64 LE][payload_len: u64 LE][payload: N bytes]`
  - Mark both `#[deprecated]` — they exist only to keep the workspace compiling during migration
- [ ] PRE-0.3: **Fix `xtask build-rust-modules` package naming**
  - Current `-p` loop applies `replace('-', "_")` which breaks `mmio-socket-bridge`
  - Fix: use `--workspace --exclude` for each non-QEMU-plugin tool package instead

**Gate**: `cargo build --workspace` exits 0. `make test-check` exits 0.

---

## [PRE-1] Hard Prune: Legacy Code Elimination (KISS / YAGNI)

> Remove dead code paths so future agents work on one coherent architecture.
> Do NOT begin P0 peripheral work until this is done — legacy paths confuse agents and hide bugs.

### L1 — Coordinator legacy cleanup
- [ ] L1.1: Delete all legacy topic-string routing from coordinator
  (`sim/chardev/*`, `sim/spi/*`, `sim/wifi/*` substring matching)
- [ ] L1.2: Simplify or delete `topics.rs` — only 4 RFC-0042 topics remain:
  `sim/ch/**`, `sim/coord/*/done`, `sim/coord/link/register/*`, `sim/network/control`
- [ ] L1.3: Simplify `message_log.rs` — remove `Protocol`-based pcap dispatch;
  log by `link_id` only (KISS: link_id is all the coordinator knows)
- [ ] L1.4: Audit `zenoh_coordinator` — if it duplicates `deterministic_coordinator` logic,
  consolidate or delete it

### L2 — Port all `decode_frame` callers to RFC-0042 VtimeIngress
- [ ] L2.1: `uart` → `VtimeIngress::new_for_link(link_id, …)`
- [ ] L2.2: `ui` (observability) → `VtimeIngress::new_for_link`
- [ ] L2.3: `sensor` → `VtimeIngress::new_for_link`
- [ ] L2.4: `ieee802154` → `VtimeIngress::new_for_link`
- [ ] L2.5: `canfd` → `VtimeIngress::new_for_link`
- [ ] L2.6: `flexray` → `VtimeIngress::new_for_link`
- [ ] L2.7: `ethernet` → `VtimeIngress::new_for_link`
- [ ] L2.8: `s32k144-lpuart` → `VtimeIngress::new_for_link`
- [ ] L2.9: After all 8 ported: **delete** the `decode_frame` / `encode_frame` shims from `virtmcu-wire`

### L3 — Wire-protocol dead code
- [ ] L3.1: Suppress or fix the 25 `#[deprecated]` warnings from `VirtmcuHandshake` in
  `core_generated.rs` — either regenerate the FlatBuffer or add a targeted `#[allow]`
- [ ] L3.2: Remove `Protocol` enum from topology if the only remaining consumer is
  `message_log.rs` after L1.3

**Gate**: `grep -r "decode_frame\|encode_frame\|chardev_tx\|chardev_rx\|base_topic" hw/rust/` → 0 matches.
`cargo build --workspace` produces 0 errors and 0 warnings on peripheral crates.

---

## [P0] Reference Peripheral: Gold Standard

> One peripheral, all three test tiers green. This is the proof-of-correctness for the
> entire RFC-0041 + RFC-0042 stack before mass migration begins.
>
> Reference: `hw/rust/examples/reference-peripheral/`

### P0.1 — Unit Tests
- [ ] P0.1.1: `cargo test -p reference_peripheral` — all unit tests pass
- [ ] P0.1.2: `cargo test -p virtmcu-wire` — all wire protocol tests pass
- [ ] P0.1.3: `cargo test -p virtmcu-qom` — BqlContext, ClosureTimer, drain teardown tests pass

**Gate**: `make test-unit` exits 0 for the three packages above.

### P0.2 — Integration Test: Unix Transport
- [ ] P0.2.1: `test_reference_ping_pong_unix` passes
  (coordinator UDS + QEMU with rebuilt `.so` plugin)
- [ ] P0.2.2: `test_shutdown_safety` passes — teardown during blocked MMIO, no sanitizer errors

**Gate**: `make test-reference-peripheral` exits 0 (unix transport path).

### P0.3 — E2E Ping Pong: Full Transport Parity
- [ ] P0.3.1: `test_reference_ping_pong_zenoh` passes (Zenoh transport coordinator)
- [ ] P0.3.2: `test_reference_ping_pong_transport_parity` passes
  (unix and zenoh produce bit-identical output)
- [ ] P0.3.3: ASAN clean on reference peripheral teardown

**Gate**: All three `reference_network` integration tests pass. `make ci-full` exits 0 for the
reference peripheral scope.

---

## [P1] Mass Peripheral Migration

*Blocked on P0.3 (reference peripheral fully green) and PRE-1 L2 (all decode_frame callers ported).*

Migrate each peripheral in this order — one at a time, gate passes before next:

1. `uart` — highest test coverage, good canary
2. `sensor` — already has smoke tests from P1 prep
3. `canfd`, `flexray`, `ethernet`, `ieee802154` — buses (similar pattern)
4. `ui` (observability), `s32k144-lpuart` — last because least critical to simulation correctness

Per-peripheral checklist (RFC-0041 + RFC-0042):
- `VtimeIngress::new_for_link(link_id, …)` for ingress (no topic strings)
- `reserve_link(link_id, size)` for egress (no `reserve(topic, …)`)
- `BqlContext` in read/write/realize signatures
- `ClosureTimer` for all timers
- `dynamic_cast_qom` for pointer casts
- `drain: VcpuDrain` + `_guard = self.drain.acquire()` in every MMIO handler
- Existing test passes before next peripheral starts

**Gate**: `grep -r "DrainToken\|deref_qom_ptr\|opaque_to_state\|decode_frame\|encode_frame" hw/rust/` → 0 matches.

---

## [P2] Hardware Expansion

*Blocked on P1 (all peripherals migrated to RFC-0041 + RFC-0042).*

| Task | Description |
|---|---|
| CAN-FD (Bosch M_CAN) | Missing register logic; frame delivery through coordinator |
| FlexRay | IRQ lines; Bosch E-Ray Message RAM; SystemC build fix |
| WiFi (802.11) | Physics gateway federate; SPI/UART co-processor |
| Thread | Depends on WiFi; SPI 802.15.4 co-processor |
| Bluetooth | nRF52840 RADIO emulation |
| Automotive Ethernet | 100BASE-T1 |

Each task: implement → write integration test → gate passes before next.

---

## [P3] Strategic Evolution (Parked)

- [ ] Enterprise SSoT Generation (YAML → TypeSpec → OpenUSD)
- [ ] Real-Time Visualization (Rust/Axum + React Flow)
- [ ] Sensor Data Replay (MCAP + `virtmcu-replay`)
- [ ] External Input Boundaries (interactive mode + replay)

---

## Ongoing Risks

| ID | Risk | Mitigation |
|---|---|---|
| R1 | QEMU patch drift | Pinned; all changes via `virtmcu-cli setup patch-qemu` |
| R2 | `decode_frame` shim format mismatch | Shim is documented as transitional; removed after L2 |
| R3 | `CoordMessage` FlatBuffer ghost references | Deleted from core.fbs; `grep CoordMessage hw/rust/` must stay 0 |
| R4 | Stale `.so` plugin | Rebuild after any source change: `ninja -C .../build-virtmcu install` |
| R5 | `icount` performance | `slaved-icount` only for sub-quantum precision; `slaved-suspend` is default |
