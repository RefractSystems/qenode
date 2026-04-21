# virtmcu Active Implementation Plan

**Goal**: Make QEMU behave like Renode — dynamic device loading, FDT-based ARM machine instantiation, and deterministic multi-node simulation.
**Primary Focus**: Binary Fidelity — unmodified firmware ELFs must run in VirtMCU as they would on real hardware.

---

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
- **The Beyonce Rule**: "If you liked it, you shoulda put a test on it." Prove bugs with failing tests before fixing.
- **Lint Gate**: `make lint` must pass before every commit (ruff, version checks, cargo clippy).

---

## 2. P0: Immediate Actions (Fixes & Hardening)

### **CRITICAL: Fix Main Branch Test Failures**
**Goal**: Restore the `main` branch to a fully green state.
1. **Diagnose**: Identify all failing tests in current CI and local `make ci-full`.
2. **Isolate**: Determine if failures are due to recent changes, environmental drift, or race conditions.
3. **Fix**: Apply surgical fixes to restore stability. **This is the top priority before any new feature work.**

### Restore Full Parallel Execution
**Goal**: Enable `pytest -n auto` without resource contention.
1. **Dynamic Resource Allocation**: Ensure UNIX sockets (QMP, UART) and Zenoh topics use dynamic ports/UUIDs.
2. **Artifact Isolation**: Use `tmp_path` for all generated DTBs, ELFs, and linker scripts.
3. **Zenoh Topic Isolation**: Use unique UUID prefixes for *every* test run.
4. **Remove `xdist_group(name="serial")`**: Once stable, remove all serial markers.

---

## 3. Active Roadmap (Dependency Order)

### [Core] Phase 3.5 — YAML Platform Description & OpenUSD 🚧
*Depends on: Phase 3 (Parser) ✅*
- [ ] Complete YAML schema validation for all current peripherals.
- [ ] Ensure `yaml2qemu.py` supports new `zenoh-chardev` and `mmio-socket-bridge` mappings.

### [Core] Phase 4 — Robot Framework & QMP Hardening 🚧
*Depends on: Phase 1 (QEMU) ✅*
- [ ] Harden `QmpBridge` for high-latency or high-load scenarios.
- [ ] Ensure virtual-time-aware timeouts are used in all integration tests.

### [Core] Phase 6 & 7 — Deterministic Multi-Node Loop 🚧
*Depends on: Phase 1 (QEMU) ✅, Phase 18 (Rust Zenoh) ✅*
- [ ] **6.5** Multi-Node Ethernet Verification (Zephyr echo samples).
- [ ] **6.6** Industry-Standard Ethernet MAC Emulation (ADR-006).
- [ ] **7.8** Finalize `zenoh-netdev` RX determinism with priority queues.

### [Hardware] Phase 20.5 — SPI Bus & Peripherals 🚧
*Depends on: Phase 19 (Rust QOM) ✅*
- [ ] **20.5.1** SSI/SPI Safe Rust Bindings in `virtmcu-qom`.
- [ ] **20.5.2** Verify PL022 (PrimeCell) SPI controller in `arm-generic-fdt`.
- [ ] **20.5.3** Implement `hw/rust/zenoh-spi` bridge.
- [ ] **20.5.4** SPI Loopback/Echo Firmware verification.

### [Hardware] Phase 27 — FlexRay (Automotive) 🚧
*Depends on: Phase 5 (Bridge) ✅, Phase 19 (Rust QOM) ✅*
- [ ] **27.1.1** Add FlexRay Interrupts (IRQ lines).
- [ ] **27.1.2** Implement Bosch E-Ray Message RAM Partitioning.
- [ ] **27.2.1** Fix SystemC build regression (CMake 4.3.1 compatibility).

### [Hardware] Phase 21 — WiFi (802.11) 🚧
*Depends on: Phase 20.5 (SPI)*
- [ ] **21.7.1** Harden `arm-generic-fdt` Bus Assignment (Child node auto-discovery).
- [ ] **21.7.2** Formalize `virtmcu-wifi` Rust QOM Proxy.
- [ ] **21.2** Implement SPI/UART WiFi Co-Processor (e.g., ATWINC1500).

### [Hardware] Phase 22 — Thread Protocol 🚧
*Depends on: Phase 20.5 (SPI), Phase 21 (WiFi)*
- [ ] **22.1** Deterministic Multi-Node UART Bus Bridge.
- [ ] **22.2** SPI 802.15.4 Co-Processor (e.g., AT86RF233).

### [Infrastructure] Phase 30 — Deep Oxidization & Testing Overhaul 🚧
*Ongoing*
- [x] **30.6** Migrate `remote-port` to Rust.
- [ ] **30.8** Comprehensive Firmware Coverage (drcov integration).
- [ ] **30.10** Unified Coverage Reporting (Host + Guest).

### [Future] Connectivity Expansion
*Depends on: Core simulation loops and bus bridges*
- [ ] **Phase 23**: Bluetooth (nRF52840 RADIO emulation).
- [ ] **Phase 24**: CAN FD (Bosch M_CAN).
- [ ] **Phase 26**: Automotive Ethernet (100BASE-T1).
- [ ] **Phase 28**: Full Digital Twin (Multi-Medium Coordination).

---

## 4. Technical Debt & Future Risks

| ID | Risk | Mitigation |
|---|---|---|
| R1 | `arm-generic-fdt` patch drift | Strictly pin QEMU version; track upstream `accel/tcg` changes. |
| R7 | `icount` performance | Only use `slaved-icount` when sub-quantum precision is mandatory. |
| R11 | Zenoh session deadlocks | Implement non-blocking shutdown in `virtmcu-zenoh` helper. |
| R14 | High MTU WiFi/Eth latency | Use lock-free MPSC channels for packet injection. |

---

## 5. Permanently Rejected / Won't Do
- Python-in-the-loop for clock sync (ADR-001).
- Windows Native Support (QEMU module loading issues).
- Generic "virtmcu-only" hardware interfaces (Violates ADR-006 Binary Fidelity).
