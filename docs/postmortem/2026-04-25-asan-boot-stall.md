# Postmortem: QEMU ASan Boot-Time Clock Stalls

## Executive Summary
Tests executing under AddressSanitizer (ASan) started failing unpredictably during the `test_phase7` and `test_phase8` milestones with `CLOCK STALL` timeout errors. The timeouts occurred exclusively at `vtime=0`. We found that the extensive overhead imposed by ASan on QEMU's TCG (Tiny Code Generator) block translation during the emulator's first few instructions caused the simulator to exceed the `zenoh-clock` peripheral's default 5-second `stall-timeout`.

## Timeline & Impact
- **Symptom:** `pytest` runs under `VIRTMCU_USE_ASAN=1` failing intermittently with:
  `RuntimeError: Node 0 reported CLOCK STALL (error=1) at vtime=0. QEMU failed to reach TB boundary within its stall-timeout.`
- **Impact:** Failed integration tests during PR validation, flakiness, and test code bloat where engineers hardcoded `stall-timeout=60000` to bypass the issue.

## Root Cause Analysis
Under normal execution, a 5,000ms wall-clock timeout is more than enough for QEMU to reach the first virtual-time quantum boundary. However, under ASan, the first execution of code requires TCG to translate ARM instructions into host x86/ARM64 instructions while instrumenting memory accesses. 
This JIT compilation is disproportionately heavy during the *very first* instructions executed (boot), taking between 10 to 20 seconds. 

The `zenoh-clock` peripheral enforced the 5-second `stall-timeout` uniformly across all quanta. Since the very first `VirtualTimeAuthority.step(0)` waits for the first quantum boundary, it triggered a stall detection before the boot sequence could complete.

## Initial (Flawed) Workaround
The initial workaround involved manually increasing the `stall-timeout` in the QEMU CLI arguments inside the Python test fixtures:
```python
extra_args = ["-S", "-device", f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router},stall-timeout=60000"]
```
### Why this was bad:
1. **Developer Burden:** Test authors had to know about ASan overhead and manually configure QEMU.
2. **Loss of Strict Deadlock Detection:** Setting a 60-second stall timeout for *every* quantum means that if a genuine deadlock occurs later in the simulation (e.g., at `vtime=1_000_000`), the test runner would hang for a full minute instead of failing fast in 5 seconds.

## Robust Architectural Fix
We eliminated the need for manual timeout adjustments by addressing the problem at the orchestrator layer in `tests/conftest.py`:
1. **Dynamic Timeout Scaling**: We introduced a mechanism that automatically intercepts the `VIRTMCU_USE_ASAN=1` environment variable.
2. **Environment Injection**: When an ASan test run is detected, the Python harness automatically injects `VIRTMCU_STALL_TIMEOUT_MS=300000` (5 minutes) into the environment *before* any QEMU instance is spawned.
3. **Seamless Inheritance**: Both the Python orchestrator's `VirtualTimeAuthority` (which calculates `_DEFAULT_VTA_STEP_TIMEOUT_S`) and the QEMU `zenoh-clock` Rust plugin automatically inherit this scaled value.

### Critique & Further Improvements
- **What went well:** The fix is entirely invisible to the end-user writing tests. Tests no longer need `stall-timeout=60000` hardcoded.
- **Why `is_first_quantum` failed:** We initially attempted to scale the timeout *only* for the first quantum (`vtime=0`) inside `zenoh-clock`. However, we discovered that ASan overhead is not isolated to the boot block; large subsequent quanta (e.g., 10M ns steps) executing new branches also trigger massive TCG compilation overhead, easily exceeding 5 seconds. Therefore, global scaling via `VIRTMCU_STALL_TIMEOUT_MS` is the only robust solution.
- **What could be better:** We still rely on a wall-clock timeout. If a genuine deadlock occurs during an ASan test, the pipeline will hang for 5 minutes instead of failing fast. A purely deterministic handshake or pausing the wall-clock timeout during TCG block translation inside QEMU would provide faster feedback. 

## Action Items
1. [x] Remove `stall-timeout=60000` hacks from `tests/test_phase7.py` and `tests/test_phase8.py`.
2. [x] Implement automatic `VIRTMCU_STALL_TIMEOUT_MS` scaling in `tests/conftest.py` upon detecting `VIRTMCU_USE_ASAN=1`.
3. [x] Reverted the failed `is_first_quantum` logic inside `zenoh-clock`.
