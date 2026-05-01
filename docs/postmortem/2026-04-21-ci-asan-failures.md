# Postmortem: VirtMCU CI & ASan Integration Failures

**Date:** April 21, 2026
**Authors:** Gemini CLI / Marcin
**Status:** Resolved
**Impact:** `make ci-full` failed locally due to ASan linking issues and C++ compilation errors. Concurrently, the primary GitHub CI pipeline experienced widespread cascading failures (15+ failed smoke tests) due to QEMU crashing on startup. 

---

## 1. Executive Summary

This incident involved multiple distinct but overlapping issues across the local and remote CI environments. The most critical failure was a configuration drift between the local QEMU build script (`scripts/setup-qemu.sh`) and the containerized QEMU build script (`docker/Dockerfile`). 

While the local environment correctly enabled Rust support, the Dockerfile omitted the `--enable-rust` flag. Consequently, the QEMU binary built in GitHub Actions lacked the necessary symbols to load our Rust-based dynamic QOM plugins (like `rust-dummy` and `zenoh-*`). When smoke tests attempted to load these plugins, QEMU crashed immediately, resulting in `EOFError`s during QMP (QEMU Machine Protocol) negotiation.

Simultaneously, local ASan/UBSan builds were failing due to missing sanitizer flags in QEMU's Rust components, and the irq_stress (SystemC Bridge) test failed due to an upstream breaking change in a tracked dependency.

---

## 2. Timeline of Investigation

1. **Initial Report:** User reported that ASan integration tests were failing and requested fixes to ensure compatibility with GitHub CI.
2. **ASan Linking Investigation:** Discovered that running `make test-asan` failed with `undefined symbol: __ubsan_handle_type_mismatch_v1`. Identified that QEMU's `meson.build` did not pass `-fsanitize=address` to Rust components.
3. **ASan Patch Created:** Authored `patches/apply_rust_asan_fix.py` to dynamically inject sanitizer flags into QEMU's Rust targets when ASan is enabled.
4. **BQL Helper Conflicts Identified:** While fixing ASan, encountered C compilation errors regarding `virtmcu_bql_locked`. Discovered infinite recursion in the FFI C wrappers and naming collisions between QEMU-injected BQL helpers and DSO wrappers.
5. **SystemC Adapter Compilation Error:** During a `ci-full` run, the irq_stress domain failed to build the SystemC adapter. Found a C++ inheritance error (`cannot convert remoteport_tlm_memory_master* to remoteport_tlm_dev*`).
6. **"Works on My Machine" Anomaly:** User noted that `make ci-full` passed locally (after caching/environment fixes), but the exact same run on GitHub failed catastrophically.
7. **GitHub CI Log Analysis:** Inspected the remote CI logs. Noted that boot_arm (Minimal Boot) passed, but advanced domains failed with `Negotiation failed: EOFError`. This indicated QEMU was crashing immediately upon startup *only* when plugins were loaded.
8. **Dockerfile vs. Local Script Drift Discovered:** Compared `scripts/setup-qemu.sh` with `docker/Dockerfile`. Found that Stage 5 (`qemu-builder`) in the Dockerfile was missing the `--enable-rust` configure flag. 
9. **Final Resolution:** Patched the Dockerfile, pinned the broken SystemC dependency, fixed C++ include orders, and deployed the changes.

---

## 3. Root Cause Analysis

### A. The Remote CI Crash (`EOFError`)
* **What Happened:** Almost all integration tests failed remotely.
* **Root Cause:** In `docker/Dockerfile`, the `../configure` command lacked the `--enable-rust` flag. The resulting QEMU binary did not export the necessary FFI symbols for Rust. When `tests/conftest.py` launched QEMU with `-device rust-dummy` (or any Zenoh plugin), QEMU attempted to `dlopen()` the shared object, failed to resolve Rust symbols, and executed `abort()`, terminating the QMP socket instantly. 
* **Why it passed locally:** Local tests (via `scripts/setup-qemu.sh`) correctly included `--enable-rust`.

### B. Local ASan / UBSan Rust Linking Failures
* **What Happened:** `make test-asan` failed to link `rust-util-tests`.
* **Root Cause:** QEMU's Meson build system natively passes `-fsanitize=address` to C compilers but lacks the logic to pass `-C link-arg=-fsanitize=address` to `rustc`. Therefore, Rust static libraries were uninstrumented, causing linker failures when combined with instrumented C libraries.

### C. SystemC Adapter Compilation Failure (SystemC)
* **What Happened:** `make -C tools/systemc_adapter` failed with class definition and conversion errors.
* **Root Cause:** `CMakeLists.txt` was tracking the `master` branch of Xilinx's `libsystemctlm-soc`. An upstream change altered the class hierarchy. Compounding this, `remote_port_adapter.cpp` had an include-order bug where `remote-port-tlm.h` (the base class definition) was included *after* the headers for derived classes.

### D. BQL Helper Symbol Conflicts
* **What Happened:** QEMU build failed with implicit declaration and redefinition errors for BQL (Big QEMU Lock) helpers.
* **Root Cause:** The Python injection script (`apply_zenoh_hook.py`) injected helpers into QEMU without proper `extern` declarations in our plugin headers. Furthermore, `virtmcu_bql_force_unlock` was calling itself recursively instead of calling the underlying QEMU function.

### E. Python `sys.path` ModuleNotFoundError
* **What Happened:** `test_proto.py` failed with `ModuleNotFoundError: No module named 'vproto'`.
* **Root Cause:** The test script appended a `pathlib.Path` object to `sys.path`. While this sometimes works in newer Python versions locally, strict environments required string paths. `sys.path.append(TOOLS_DIR)` failed silently to resolve, leaving the module undiscoverable.

---

## 4. Resolution & Fixes Applied

1. **Dockerfile Parity:** 
   * Added `--enable-rust` and `--enable-debug` to `docker/Dockerfile` Stage 5 to achieve parity with `setup-qemu.sh`.
2. **ASan Rust Patch:** 
   * Created `patches/apply_rust_asan_fix.py` to inject `add_project_arguments('-C', 'link-arg=-fsanitize=address', language: 'rust')` into QEMU's `meson.build`.
3. **BQL Abstraction Hardening:** 
   * Renamed injected QEMU helpers to `virtmcu_safe_bql_force_unlock` to avoid namespace collisions.
   * Fixed the infinite recursion in `ffi.c`.
   * Added explicit `extern` prototypes to `ffi.h`.
4. **SystemC Adapter Stability:** 
   * Pinned `systemctlm_soc` to commit `7aba4f60...` in `CMakeLists.txt`.
   * Reordered `#include` statements in `remote_port_adapter.cpp`.
   * Improved `Makefile` robustness by adding a `mkdir -p build` dependency.
5. **CI Orchestration Fixes:** 
   * Cast `Path` objects to strings via `str(TOOLS_DIR)` before appending to `sys.path`.
   * Synchronized `Makefile` Docker targets (`test-integration-docker`) to use identical `--user $(id -u):$(id -g)` and `CARGO_HOME` mappings to prevent root-owned cache collisions.

---

## 5. Action Items & Lessons Learned

* **Avoid `master` tags in FetchContent:** Always pin external dependencies (like `libsystemctlm-soc`) to specific Git SHAs or release tags to prevent unannounced upstream breaks.
* **Single Source of Truth for Builds:** The configuration flags for QEMU were duplicated between `scripts/setup-qemu.sh` and `docker/Dockerfile`. In the future, we should source a shared `.env` or configuration file to prevent environment drift.
* **Trust the Pipeline Breakdown:** The fact that boot_arm passed while dynamic_plugin failed was the exact clue needed to diagnose a dynamic plugin loading issue rather than a fundamental QEMU compilation failure.