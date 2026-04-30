> **Note:** This postmortem is archived. The failure modes described here are now structurally prevented by project guardrails (e.g., FlatBuffers, RAII BQL wrappers, strict linters).\n
# Postmortem: ARCH-8 Synchronization Barrier Debugging

## Date
April 28, 2026

## Authors
Gemini CLI / VirtMCU Engineering

## Status
Resolved

## Executive Summary
During the implementation of the ARCH-8 TA/Coordinator Synchronization Protocol, the integration tests repeatedly failed despite the core barrier logic appearing correct. The debugging session revealed two compounded issues: a silent QEMU plugin load failure due to an exported Rust symbol missing `#[no_mangle]`, and a `NameError` in the Python test teardown that masked the original failure. This incident highlights the need for stricter FFI visibility checks and more robust test cleanup procedures.

## Root Cause Analysis

### 1. The FFI Symbol Visibility Bug
To implement the clock advance barrier, the `virtmcu-clock` device required a hook into the QEMU vCPU halt loop (`clock_cpu_halt_cb`). This function was defined in Rust (`hw/rust/backbone/clock/src/lib.rs`) as `extern "C"` to be callable from QEMU's C core.
* **The Error:** The `#[no_mangle]` attribute was omitted.
* **The Mechanism:** Rust mangled the symbol name during compilation. When QEMU attempted to dynamically load (`dlopen`) `hw-virtmcu-clock.so`, the dynamic linker could not resolve the `clock_cpu_halt_cb` symbol. QEMU instantly aborted with an "undefined symbol" error.

### 2. The Test Teardown Masking Bug
The integration test (`tests/test_arch8_coordinator_sync.py`) orchestrates a complex multi-node Zenoh setup.
* **The Error:** The test's `finally` block attempted to clean up Zenoh subscriptions by calling `done_sub.undeclare()` and `uart_sub.undeclare()`. However, these variables were never assigned; the subscription declarations were awaited as anonymous lambdas.
* **The Mechanism:** When QEMU crashed (due to Bug 1), the QMP connection failed, triggering the `finally` block. The `finally` block threw a `NameError`. In Python's async exception handling, this secondary error bubbled up, obscuring the original `ConnectError` and the QEMU `STDERR` output. 

## Timeline of Events
1. **Implementation:** Barrier logic added to `ZenohClockResponder::send_ready` and Python coordinator loop.
2. **Initial Test Run:** Test failed via timeout/cancellation. Assertion output suggested a timing violation, but elapsed time (e.g., 0.37s) actually satisfied the condition (> 0.1s).
3. **Investigation:** Added verbose logging (`sim_info!` and `print`) to track Zenoh signals (`done` and `start`).
4. **Discovery 1 (The Mask):** Realized the Python test was failing with an obscured traceback. Found and fixed the `NameError` in the `finally` block by explicitly assigning the subscription variables.
5. **Discovery 2 (The Root):** With the `NameError` fixed, the true error was revealed: `EOFError` on QMP connection. Running QEMU manually with `-device help` exposed the `undefined symbol: clock_cpu_halt_cb` error.
6. **Resolution:** Added `#[no_mangle]` to the Rust function. Recompiled the DSO via `ninja`. 
7. **Verification:** Test passed successfully, confirming the ARCH-8 barrier correctly delays the TA clock advance until coordinator delivery is complete.
8. **Linting Cleanup:** Fixed resulting `mypy` type inference errors caused by passing lambdas to `asyncio.to_thread` by converting them to explicitly typed inner functions.

## Lessons Learned & Action Items

### 1. Always Check QEMU STDERR First
When `QmpBridge` fails to connect (`EOFError`), it almost always means QEMU failed to initialize before opening the socket. **Action:** We must train ourselves (and AI agents) to immediately inspect the first 5 lines of QEMU `STDERR` for `failed to open module` or `undefined symbol` whenever a test fails during `start_emulation()`.

### 2. FFI Symbol Strictness (The 3-Layer Defense)
`extern "C"` does not guarantee symbol visibility to `dlopen` in Rust; `#[no_mangle]` is mandatory. We have implemented a 3-layer defense mechanism:
* **Level 1 (Binary Lint):** Added `scripts/verify-exports.py`, which is run during `make lint`. It uses `nm -D` to proactively verify that required un-mangled FFI symbols (like `clock_cpu_halt_cb`) exist in the compiled `.so` plugins.
* **Level 2 (Explicit Macro):** Introduced the `virtmcu_export!` macro in `virtmcu-api`. This macro explicitly wraps FFI exports and automatically applies `#[no_mangle] extern "C"`, removing the possibility of human error when declaring QEMU-facing hooks.
* **Level 3 (Test Fail-Fast):** Updated the QEMU launcher in `tests/conftest.py` to continuously scan QEMU's `STDERR` stream during startup. If "failed to open module", "undefined symbol", or "not a valid device model name" is detected, the launcher immediately raises a descriptive `RuntimeError`, bypassing the QMP timeout and `EOFError` masking.

### 3. Safe Test Teardown and Deadlock Prevention
`finally` blocks in `pytest` must be defensive. If an exception occurs *before* a resource is allocated, the `finally` block will throw a `NameError` trying to clean it up.
* **Action:** Always initialize teardown variables to `None` at the start of the test, and check `if resource is not None:` in the `finally` block.
* **Teardown Deadlocks:** We discovered that background `asyncio` tasks reading QEMU output streams could deadlock the event loop if not explicitly cancelled during teardown. 
* **Action:** We added explicit task cancellation (`task.cancel()`) for stream readers and lowered the `asyncio.wait_for` timeouts from `5.0s` to `0.5s` for subprocess termination to ensure tests exit rapidly.

### 4. Fast Iteration Loop
Running `make build` for a single missing attribute wastes minutes. **Action:** The usage of `ninja -C third_party/qemu/build-virtmcu hw-virtmcu-<plugin>.so` proved highly effective for a sub-second edit-compile-test loop and is now documented in `DEBUG_PLAN_ARCH8.md`.
