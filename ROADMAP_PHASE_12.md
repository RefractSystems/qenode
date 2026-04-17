# virtmcu Roadmap: Phase 12 Upstream Improvements

This document outlines the critical architectural and usability improvements for the \`virtmcu\` engine, identified during the FirmwareStudio Phase 11.4 integration. These changes aim to eliminate silent failures, improve debuggability, and stabilize the co-simulation timing model.

---

## 1. Unambiguous Error Reporting in \`zenoh-clock\` [P0]

**Problem:** \`zenoh-clock\` returns a generic "Timeout" error for both Zenoh connection failures and QEMU execution stalls. This makes it impossible to distinguish between a networking bug and a firmware performance issue.

**Goal:** Provide distinct error codes and proactive logging.

### Tasks:
- [x] **Proactive Connection Logging:** `fprintf(stderr, ...)` added in `realize` for session open failure and successful connect. Missing-router path now emits a clear WARNING with container-specific guidance.
- [x] **Specific Error Payloads:** `ClockReadyPayload` updated with `error_code` field (`0`=OK, `1`=STALL).
    - `0` = OK
    - `1` = INTERNAL_STALL (QEMU didn't reach TB boundary within 60 s)
    - `2` = ZENOH_ERROR — emitted by `on_query` for: (a) query arrives with no payload, (b) `ClockAdvancePayload` is truncated/malformed. `z_query_reply` failure is logged but cannot carry error_code=2 (the transport is already gone). Session-open failure exits via `error_setg` before any queryable exists — TimeAuthority observes a connection reset, which is the correct signal.

### Verification:
1. Launch QEMU with a wrong `router=` parameter. Verify `stderr` contains a clear connection error. ✓
2. Launch QEMU with `-icount` disabled and verify the TimeAuthority receives a specific "STALL" error rather than a generic timeout. ✓

---

## 2. \`yaml2qemu\` Output Validation [P0]

**Problem:** \`yaml2qemu\` can generate a \`.dtb\` that QEMU loads, but if a device is missing its memory mapping (due to incorrect YAML or \`dtc\` dropping nodes), QEMU fails silently at runtime with Data Aborts.

**Goal:** Ensure the generated Device Tree actually contains the expected peripherals.

### Tasks:
- [x] **Post-Compilation Check:** `validate_dtb()` runs `dtc -I dtb -O dts` after every successful compile.
- [x] **Mapping Assertion:** Checks for `name@address` DTS node format (address-qualified) to catch wrong-address mappings, not just name presence.
- [x] **Fatal Exit:** Exits with code `1` and prints missing device names. Missing `dtc` binary is also a fatal error (not a silent skip).

### Verification:
1. Create a malformed YAML where a device type is unknown to FdtEmitter. Verify `yaml2qemu` fails and reports the missing mapping. ✓ (covered by `tests/test_yaml_validation.py`)

---

## 3. MMIO Bridge Protocol: Offsets vs. Absolute Addresses [P1]

**Problem:** \`mmio-socket-bridge\` currently delivers absolute physical addresses to the socket. This forces the external model (Python/SystemC) to be coupled to the specific board address map, preventing modular peripheral reuse.

**Goal:** Deliver base-relative offsets to the socket server.

### Tasks:
- [x] **Address Translation:** `mmio-socket-bridge.c` passes `addr` from `MemoryRegionOps` directly — QEMU already delivers a region-relative offset. The `base-addr` property is used solely for `sysbus_mmio_map()` to place the device in guest address space.
- [x] **Protocol Documentation:** `CLAUDE.md` Key Constraints section updated to state offsets are delivered.

### **CRITICAL WARNING:**
Changing this is a **BREAKING CHANGE**. You MUST update \`studio_server.py\` in the \`FirmwareStudio\` repository to remove any \`addr &= 0xFFF\` masking logic simultaneously.

### Verification:
1. Run the \`pendulum\` demo. Verify the MMIO bridge receives \`0x0C\` and \`0x14\` rather than \`0x1000000C\` and \`0x10000014\`.

---

## 4. Documentation: Timing Model and WFI Behavior [P1]

**Problem:** The behavior of \`WFI\` (Wait For Interrupt) under \`-icount\` and the interaction between MMIO socket blocking and \`icount\` advancement is undocumented.

**Goal:** Provide clear guidance for firmware developers.

### Tasks:
- [x] **Document WFI Interaction:** `CLAUDE.md` § Timing Model covers WFI in `slaved-suspend` mode.
- [x] **Document MMIO Blocking:** `CLAUDE.md` § Timing Model explicitly states vCPU is Halted during socket wait and that icount does not advance.
- [x] **Create `docs/TIMING_MODEL.md`:** Standalone reference document covering clock modes, wire protocol, error codes, virtual-time advancement rules, quantum boundary sequence, BQL rules, and performance notes. Cross-references `TIME_MANAGEMENT_DESIGN.md` for tutorial content.

---

## 5. Summary Table

| Task | File(s) | Type | Impact |
| :--- | :--- | :--- | :--- |
| **Clock Errors** | \`hw/zenoh/zenoh-clock.c\` | Bug Fix | High |
| **DTB Validation** | \`tools/yaml2qemu.py\` | Feature | High |
| **Offset Protocol** | \`hw/misc/mmio-socket-bridge.c\` | API Change | Medium (Breaking) |
| **Timing Docs** | \`docs/TIMING_MODEL.md\` | Docs | Medium |

---
