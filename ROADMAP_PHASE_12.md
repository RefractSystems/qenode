# virtmcu Roadmap: Phase 12 Upstream Improvements

This document outlines the critical architectural and usability improvements for the \`virtmcu\` engine, identified during the FirmwareStudio Phase 11.4 integration. These changes aim to eliminate silent failures, improve debuggability, and stabilize the co-simulation timing model.

---

## 1. Unambiguous Error Reporting in \`zenoh-clock\` [P0]

**Problem:** \`zenoh-clock\` returns a generic "Timeout" error for both Zenoh connection failures and QEMU execution stalls. This makes it impossible to distinguish between a networking bug and a firmware performance issue.

**Goal:** Provide distinct error codes and proactive logging.

### Tasks:
- [ ] **Proactive Connection Logging:** In \`hw/zenoh/zenoh-clock.c\`, add explicit \`fprintf(stderr, ...)\` logs during the \`realize\` phase if the Zenoh session fails to open or if the queryable declaration fails.
- [ ] **Specific Error Payloads:** Update the Zenoh reply payload to include an error type field.
    - \`0\` = OK
    - \`1\` = INTERNAL_STALL (QEMU didn't reach TB boundary)
    - \`2\` = ZENOH_ERROR (Underlying transport failure)

### Verification:
1. Launch QEMU with a wrong \`router=\` parameter. Verify \`stderr\` contains a clear connection error.
2. Launch QEMU with \`-icount\` disabled and verify the TimeAuthority receives a specific "STALL" error rather than a generic timeout.

---

## 2. \`yaml2qemu\` Output Validation [P0]

**Problem:** \`yaml2qemu\` can generate a \`.dtb\` that QEMU loads, but if a device is missing its memory mapping (due to incorrect YAML or \`dtc\` dropping nodes), QEMU fails silently at runtime with Data Aborts.

**Goal:** Ensure the generated Device Tree actually contains the expected peripherals.

### Tasks:
- [ ] **Post-Compilation Check:** After generating the \`.dtb\`, \`yaml2qemu\` must run \`dtc -I dtb -O dts\` on the result.
- [ ] **Mapping Assertion:** Grep/Parse the DTS output to verify that every peripheral defined in the input YAML (that isn't a \`zenoh-chardev\`) has a corresponding node with a \`reg\` property at the expected address.
- [ ] **Fatal Exit:** If any device is missing, \`yaml2qemu\` must exit with code \`1\` and print the names of the missing devices.

### Verification:
1. Create a malformed YAML where a device name is illegal. Verify \`yaml2qemu\` fails and reports the missing mapping.

---

## 3. MMIO Bridge Protocol: Offsets vs. Absolute Addresses [P1]

**Problem:** \`mmio-socket-bridge\` currently delivers absolute physical addresses to the socket. This forces the external model (Python/SystemC) to be coupled to the specific board address map, preventing modular peripheral reuse.

**Goal:** Deliver base-relative offsets to the socket server.

### Tasks:
- [ ] **Address Translation:** In \`hw/misc/mmio-socket-bridge.c\`, subtract \`s->base_addr\` from the \`hwaddr addr\` before packing it into the \`mmio_req\` struct.
- [ ] **Protocol Documentation:** Update \`virtmcu_proto.h\` and associated docs to reflect that \`addr\` is now an offset.

### **CRITICAL WARNING:**
Changing this is a **BREAKING CHANGE**. You MUST update \`studio_server.py\` in the \`FirmwareStudio\` repository to remove any \`addr &= 0xFFF\` masking logic simultaneously.

### Verification:
1. Run the \`pendulum\` demo. Verify the MMIO bridge receives \`0x0C\` and \`0x14\` rather than \`0x1000000C\` and \`0x10000014\`.

---

## 4. Documentation: Timing Model and WFI Behavior [P1]

**Problem:** The behavior of \`WFI\` (Wait For Interrupt) under \`-icount\` and the interaction between MMIO socket blocking and \`icount\` advancement is undocumented.

**Goal:** Provide clear guidance for firmware developers.

### Tasks:
- [ ] **Document WFI Interaction:** Research and document if \`WFI\` in \`-icount\` mode correctly pauses virtual time and yields the host thread. This is a prerequisite for using the ARM Generic Timer.
- [ ] **Document MMIO Blocking:** Explicitly state that the vCPU is Halted while \`mmio-socket-bridge\` is waiting for a socket response. Clarify if \`icount\` advances or pauses during this window.
- [ ] **Create \`docs/TIMING_MODEL.md\`:** A central document explaining these nuances to users.

---

## 5. Summary Table

| Task | File(s) | Type | Impact |
| :--- | :--- | :--- | :--- |
| **Clock Errors** | \`hw/zenoh/zenoh-clock.c\` | Bug Fix | High |
| **DTB Validation** | \`tools/yaml2qemu.py\` | Feature | High |
| **Offset Protocol** | \`hw/misc/mmio-socket-bridge.c\` | API Change | Medium (Breaking) |
| **Timing Docs** | \`docs/TIMING_MODEL.md\` | Docs | Medium |

---
