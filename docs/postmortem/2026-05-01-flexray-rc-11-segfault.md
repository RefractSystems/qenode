# Case Study: The FlexRay SIGSEGV (Cascading Silent Failures)

## 1. Problem Statement
The integration test for the FlexRay peripheral was failing with a generic `rc=-11` (SIGSEGV) exit code from QEMU. No error message was printed to the console, and the guest firmware never appeared to start.

## 2. Hypothesis
Initial hypothesis: "There is a bug in the FlexRay Rust plugin that triggers a memory corruption during initialization."

## 3. Evidence
*   **ASan Output:** Attaching AddressSanitizer revealed a NULL pointer dereference inside `error_vprepend()` in QEMU's core utility code.
*   **QMP Inspection:** Attempting to `qom-list` the peripheral showed it was missing from the object tree.
*   **DTS Review:** The CPU node was missing the `memory` phandle.

## 4. Root Cause
The "SIGSEGV" was actually the result of **three stacked defects**:
1.  **Missing CPU Memory:** The CPU had no address space, causing it to fault before `main()`.
2.  **Four-way Name Mismatch:** The peripheral's name was inconsistent across Rust, DTS, Meson, and pytest. QEMU silently skipped loading the device.
3.  **QEMU Error Path Bug:** When a module failed to load, QEMU's error reporting code itself crashed because it was passed a NULL pointer.

## 5. The Fix
*   Aligned all names to `flexray`.
*   Added the missing `memory` phandle to the DTS.
*   Patched QEMU to provide a readable error message instead of crashing when a module is missing.
*   Implemented `check-qom-alignment.py` to catch name mismatches at lint time.

## 6. Takeaway: The "Fail Loudly" Principle
A system with multiple layers of abstraction must fail loudly at the first point of failure. If the name mismatch had been caught by a linter, the engineer would have saved 3 days of debugging a "ghost" SIGSEGV.

## 7. Exercise
**The Saboteur:** Open `tests/fixtures/guest_apps/flexray_bridge/platform.dts`. Change the `compatible` string of the FlexRay device to `"broken-name"`. Run the test.
*   Does it fail at lint time?
*   What is the error message in the QEMU logs?
*   How would you fix this using only the `qom-list` command?
