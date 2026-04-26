# CLAUDE.md — virtmcu Project Context

> [!IMPORTANT]
> **ENTERPRISE QUALITY MANDATE**: All agents (Claude, Gemini, etc.) must produce only enterprise-grade code. No "AI-style" shortcuts, no suppressed lints (`#[allow]`, `noqa`, `// @ts-ignore`), and no bypassing the type system. Every change must be surgically precise, idiomatically perfect, and backed by empirical test evidence.

This file is read automatically by Claude Code and Gemini CLI at session start
(\`GEMINI.md\` is a symlink to this file — maintain only this one).
Update it when architectural decisions change or new constraints are discovered.

---

## TOP PRIORITY: Binary Fidelity

**The same firmware ELF that runs on a real MCU must run unmodified in VirtMCU.**

This is the non-negotiable design constraint from which everything else follows:
- No virtmcu-specific startup code, linker sections, or compile-time flags in firmware.
- Peripherals mapped at the **exact** base addresses the real MCU datasheet specifies.
- Register layouts, reset values, and interrupt numbers must match physical silicon.
- \`zenoh-clock\` and all co-simulation infrastructure are **invisible to the firmware** — they operate at the QEMU level with no guest MMIO exposure.

Any feature that requires firmware modification to work in VirtMCU is a bug in VirtMCU, not a firmware problem. See [ADR-006](docs/ADR-006-binary-fidelity.md) for the full rationale and enforcement rules.

---

## What This Project Is

**virtmcu** is a **deterministic multi-node firmware simulation framework** built on QEMU.
Specifically, it provides:
1. **Dynamic QOM device plugins** (.so shared libraries).
2. **arm-generic-fdt machine** — ARM machines defined by Device Tree.
3. **Native Zenoh QOM plugin** (`hw/rust/`) — deterministic clock and I/O.
4. **yaml2qemu** — Parses OpenUSD-aligned YAML and emits \`.dtb\` + CLI.

---

## Clock Synchronization Model

All clock behaviour is controlled by \`zenoh-clock\`:

| Mode | How to invoke | When to use |
|---|---|---|
| \`standalone\` | No \`-device zenoh-clock\` | Rapid development, logic testing. |
| \`slaved-suspend\` | \`-device zenoh-clock,mode=slaved-suspend\` | **Default.** Deterministic co-simulation. |
| \`slaved-icount\` | Same + \`-icount shift=0,align=off,sleep=off\` | Sub-quantum timing precision (PWM, µs). |

### Error Codes (sim/clock/advance/{id} Reply)
- `0` (OK): Quantum completed successfully.
- `1` (STALL): QEMU failed to reach TB boundary within the stall timeout (default **5 s**; set `stall-timeout=<ms>` on the device — ASan CI uses 300 000 ms via `VIRTMCU_STALL_TIMEOUT_MS`).
  - **CRITICAL NOTE:** QEMU **does not exit** on a stall. It logs "STALL DETECTED", replies with error code 1, and stays alive. The Python orchestrator is fully responsible for terminating the QEMU process if recovery is not desired.
  - **Stall-Timeout Contract (agents MUST follow):**
    1. **Never hardcode `stall-timeout=<ms>` in test `extra_args`** unless the test is specifically validating stall detection behaviour (e.g., `test_phase7_clock_stall`). Hardcoding a short value causes ASan builds to spuriously report stalls.
    2. **When no explicit `stall-timeout` is set**, `zenoh-clock` reads `VIRTMCU_STALL_TIMEOUT_MS` from the environment (CI: 120 000 ms; ASan CI: 300 000 ms; default: 5 000 ms).
    3. **The Python VTA step timeout** (`VirtualTimeAuthority.step(timeout=...)`) must always be *greater* than the QEMU stall timeout to guarantee QEMU can reply with STALL before Python gives up. `conftest.py` computes this automatically from `VIRTMCU_STALL_TIMEOUT_MS`. Do not pass explicit timeouts shorter than `VIRTMCU_STALL_TIMEOUT_MS + 10 s` unless the test intentionally probes timeout behaviour.
    4. **ASan overhead**: under ASan/UBSan, QEMU executes TCG blocks significantly slower. Tests that advance large virtual-time quanta (e.g., 100 ms) can legitimately take several minutes of wall-clock time. This is expected and correct — do NOT "fix" it by increasing `stall-timeout` inline or by swallowing `RuntimeError` with a bare `except RuntimeError: pass`.
- `2` (ZENOH_ERROR): Transport layer failure.

---

## Timing Model and Constraints

### 1. MMIO Socket Blocking
When using \`mmio-socket-bridge\`, every MMIO read/write blocks the QEMU TCG thread in a synchronous socket syscall.
- **CPU State**: The emulated CPU is **Halted** while waiting for the server.
- **icount Advancement**: Virtual time does NOT advance while blocked in a bridge call.
- **Latency Impact**: High bridge latency can cause clock stalls. Ensure the socket server is performant.

### 2. FFI Layout Verification (The "FFI Gate")
To prevent segmentation faults caused by layout drift between C and Rust:
- **Mandate**: All core QOM structs in Rust MUST have corresponding \`assert!\` checks for both \`size_of\` and \`offset_of\`.
- **Mandate**: Before modifying or implementing a Rust FFI struct, you MUST verify the ground truth from the QEMU binary using \`./scripts/check-ffi.py\`.
- **Mandate**: If the QEMU binary is unavailable or stale, run \`make build\` before running \`make check-ffi\`.
- **Mandate**: Use \`./scripts/check-ffi.py --fix\` to automatically synchronize Rust assertions with the binary ground truth.

### 3. Plugin Staleness Prevention
- **Mandate**: Always prioritize build artifacts over installed binaries during development.
- **Mandate**: \`scripts/run.sh\` is configured to find the freshest \`.so\` plugins in the build tree. Do NOT manually override \`QEMU_MODULE_DIR\` unless strictly necessary.

### 4. Simulation Hygiene & Parallel Safety
To ensure tests are deterministic and do not interfere with each other:
- **Mandate**: NEVER use hardcoded ports (e.g., 7447) or fixed socket paths in tests. Always use \`scripts/get-free-port.py\` or \`tempfile.mkdtemp()\`.
- **Mandate**: Parallel test execution (\`pytest -n auto\`) is the standard. All fixtures must be isolated.
- **Mandate**: Multiple agents/developers CAN run native tests concurrently on the same machine, provided they work in **separate cloned workspace directories**. \`scripts/cleanup-sim.sh\` is heavily optimized to be **Workspace-Scoped**—it inspects \`/proc/<pid>/cwd\` and \`/proc/<pid>/cmdline\` to ONLY kill orphaned processes originating from the active workspace, leaving other agents' simulations completely untouched.

### 5. WFI (Wait For Interrupt) behavior
- In \`slaved-suspend\`, virtual time advances while the CPU is in WFI.
- The next quantum boundary will still trigger a clock-halt even if the CPU is idling.
- **Best Practice**: Use ARM Generic Timer interrupts at 100Hz rather than tight polling loops for control.

---

## Key Constraints

- **MMIO Delivery**: \`mmio-socket-bridge\` delivers **relative offsets** to the socket. External models should NOT include the base address in their match logic.
- **DTB Validation**: \`yaml2qemu\` validates that every peripheral defined in YAML is correctly mapped in the output DTB. If a mapping is missing, build will fail.
- **SysBus Mapping**: Devices added via \`-device\` only (not in YAML) are **NOT mapped** into guest memory. They will cause Data Aborts.

---

## Development Efficiency

- **Avoid Redundant `make setup`**: Only run `make setup` for the initial environment setup or if QEMU dependencies change. It applies core patches that can trigger massive rebuilds.
- **Use `make build` for Incremental Changes**: For standard changes to `hw/` peripherals, `make build` is sufficient and much faster.
- **Targeted Ninja Builds**: For the fastest turnaround when working on a specific peripheral, run ninja directly on the module target (ensure the build directory matches your `VIRTMCU_USE_ASAN` setting):
  `ninja -C third_party/qemu/build-virtmcu$( [ "$VIRTMCU_USE_ASAN" = "1" ] && echo "-asan" ) hw-virtmcu-<name>.so`
- **Pre-installed QEMU**: In many environments, QEMU is pre-installed at `/opt/virtmcu`. You only need to build if you are modifying the emulator or its peripheral plugins.

---

## Directory Structure

```
virtmcu/
├── hw/                         # Rust QOM peripheral models
│   └── rust/
│       ├── mmio-socket-bridge/ # Offset-based Unix socket bridge
│       └── zenoh-clock/       # Clock sync with error reporting (Native Rust)
├── tools/
│   └── yaml2qemu.py            # YAML -> DTB transpiler with validation
└── docs/                       # Human-readable documentation
```

## Dependency & Version Control

- **Centralized Versions**: Agents MUST adhere to the versions defined in the `BUILD_DEPS` file for QEMU, Zenoh, and other core dependencies.
- **Verification**: Before suggesting or implementing upgrades, verify the current pinned versions in `BUILD_DEPS` and `requirements.txt`.
- **Package Management**: Prefer `uv` (e.g., `uv pip`, `uv run`) over standard `pip` or system package managers for all Python package management and tool installations (like CMake) due to its speed and conflict resolution.

---

## Language Selection Policy (ADR-013)

| Component | Language | Rule |
| :--- | :--- | :--- |
| **Sim Loop** | **Rust** | **NATIVE ONLY.** All core plugins migrated to Rust. |
| **Physics/SystemC** | **C++** | Standard for TLM-2.0 / MuJoCo. |
| **Tooling/Parsing** | **Python** | Out-of-band only. |
| **Telemetry** | **Rust** | Direct FlatBuffers/Zenoh integration. |

**Banned:** Python in the hot simulation loop (MMIO/Clock/Netdev).
**Completed:** Core migration of `hw/zenoh/*.c` to native Rust (Phase 18 & 19).

---

## Bifurcated Testing Strategy (Rust + Python)

We adhere to a strict **Bifurcated Testing Strategy** to maximize performance, safety, and orchestration reliability.

| Layer | Language | Framework | Purpose |
| :--- | :--- | :--- | :--- |
| **White-Box Internals** | **Rust** | `cargo test` | State machines, memory layout validation (via `bindgen`), protocol parsing, lock-free queues, and FFI boundaries. |
| **Black-Box Orchestration** | **Python** | `pytest` + `asyncio` | Multi-process orchestration (QEMU + Zenoh + TimeAuthority), QMP integration, UART verification, topology setup, and end-to-end regression testing. |
| **Thin CI Wrappers** | **Bash** | `make test` / `.sh` | Entry points *only*. Bash scripts must never manage multi-process orchestration. They should merely be 2-3 line scripts that invoke `pytest` or `cargo test`. |

### Strict Rules for Parallel Execution (Out-of-the-Box Ready)

Our test suites run in massive parallel (using `pytest -n auto`). All future tests (developed by humans or agents) **MUST** be designed "out of the box" for parallel execution isolation. Violating these rules will cause flaky builds and port collisions.

1. **NO Hardcoded Ports/Endpoints:**
   - **BANNED:** Hardcoding `tcp/127.0.0.1:7447`, `7450`, or any fixed port in Python scripts, YAML, or bash files.
   - **REQUIRED:** Use dynamic port allocation. In Python `pytest`, request the `zenoh_router` fixture (or use `scripts/get-free-port.py` in bash) and inject the port dynamically via `sed`, string formatting, or command-line arguments.
2. **NO Hardcoded Temporal Paths:**
   - **BANNED:** Generating output files (e.g., `.dtb`, `.yaml`, `.cli`) directly into the `workspace_root` or shared test directories (e.g., `/tmp/phase3/out.dtb`). This causes workers to overwrite or delete each other's files.
   - **REQUIRED:** Always use the `tmp_path` fixture in `pytest` (or `mktemp -d` with cleanup in bash) to create a highly isolated directory for all generated test artifacts.
3. **NO Random Value Collisions:**
   - **BANNED:** Using `random.randint()` or generic UUIDs to generate supposedly "unique" node IDs or shared memory paths inside tests.
   - **REQUIRED:** Use deterministic uniqueness tied to the isolated execution environment (e.g., `os.getpid()`, the pytest `worker_id`, or `tmp_path` strings) to guarantee absolute isolation between parallel runners without relying on PRNG chance.
4. **NO Manual Process Management:**
   - **BANNED:** Spawning complex background daemons (like `cargo run ... zenoh_coordinator`) manually inside test bodies. This causes severe build-lock contention in Rust and port leakage.
   - **REQUIRED:** Use centralized, reusable `pytest` fixtures (e.g., `zenoh_coordinator`, `qemu_launcher`) that guarantee clean provisioning, deterministic routing, and automated teardown.
5. **Test Scope Restriction:**
   - **Mandate:** `pytest` is strictly scoped to the `tests/` directory via `pyproject.toml`. Do NOT place `.py` test files in `test/phase*/` directories as they will not be discovered.
6. **Robust Binary Resolution:**
   - **Mandate:** When calling Rust tool binaries from Python tests (e.g., `zenoh_coordinator`, `resd_replay`), always check both the workspace target directory (`target/release/`) and the tool-specific target directory (`tools/<tool_name>/target/release/`) to handle variations in how `cargo build` vs `cargo build -p` outputs artifacts.

**Mandates:**
- Never write complex setup/teardown loops in Bash. If a test requires booting QEMU alongside a background process (e.g., `zenoh_coordinator`), it **must** be written in Python using `pytest` fixtures to ensure safe teardown and prevent zombie processes.
- Internal logic (like CSMA/CA backoffs, flatbuffer serialization, struct alignment) **must** be tested natively in Rust using `#[test]`, bypassing QEMU boot entirely where possible.

### Pro-Tips for Test Construction (Parallel & Flake-Free)
- **The `-S` Boot Freeze:** When expecting immediate UART output, always pass `-S` to QEMU via `extra_args` and call `await bridge.start_emulation()` ONLY AFTER setting up your UART subscribers or `wait_for_line` calls. This prevents the firmware from executing before the test runner is fully attached.
- **Dynamic Configuration Injection:** Never commit temporary test configuration files. Use Python's `shutil.copy` to copy baseline files to `tmp_path`, then use `sed` (via `subprocess.run`) or Python `replace()` to dynamically inject the `zenoh_router` endpoint into the `.yaml` or `.dts` file before compiling it.
- **Bash Script Port Fallbacks:** All `test/*/*.sh` smoke tests MUST accept a port as `$1` and fallback to a dynamic port if not provided:
  ```bash
  PORT=${1:-0}
  if [ "$PORT" -eq 0 ]; then
      PORT=$(python3 "$WORKSPACE_DIR/scripts/get-free-port.py")
  fi
  ```
- **Time Authorities:**
  - Use `VirtualTimeAuthority` for multi-node tests that require driving multiple clock queryables simultaneously.
  - Use the legacy `TimeAuthority` for older single-node tests to maintain backward compatibility.
- **Selective Cleanup:** Use `scripts/cleanup-sim.sh --filter <pattern>` if you need to cleanly kill processes specific to a test run without nuking other parallel workers operating in the same workspace.

---

## Production Engineering Mandates

To ensure the highest level of professional software engineering, all agents MUST adhere to these standards:

### 1. Environment Agnosticism (Zero Hardcoded Paths)
- **NO absolute paths** (e.g., `/Users/marcin/...`) or user-specific home directory references.
- Use **relative paths** based on the project root or the current file's location.
- Use platform-appropriate path joining (e.g., `os.path.join` in Python, `path::PathBuf` in Rust, `std::filesystem` in C++).
- Leverage environment variables for system-specific configuration.
- **Environment Context:** All agents operate within a **devcontainer**. `localhost` ALWAYS refers to the container environment, where all tools, dependencies (like QEMU), and services (like Zenoh) are managed. Never assume access to the physical host machine's toolchain.

### 2. Explicit Constants (NEVER Use Magic Numbers!)
- **BANNED:** Inline literal numbers or "magic numbers" in the code.
- **REQUIRED:** If a number needs to be entered (e.g., `1024`), you MUST create a `const` descriptive variable with a comment explaining what it is and why it has that value, and then use that variable.
- Example: Instead of `buffer = [0; 1024]`, do:
  ```rust
  /// Standard payload size for the bridge
  const MAX_PAYLOAD_SIZE: usize = 1024;
  let buffer = [0; MAX_PAYLOAD_SIZE];
  ```
- Group related constants in configuration files or dedicated `constants` modules.

### 3. Verification & TDD (The "Beyonce Rule")
- **Prove-It Pattern:** For every bug fix, you MUST write a failing test that reproduces the bug BEFORE implementing the fix.
- **Incremental Implementation:** Every feature or change must include corresponding unit or integration tests.
- **Surgical Edits:** Keep changes focused. A single logical change per commit/PR. Separate refactoring from behavior changes.

### 4. Quality & Security Gates
- **Multi-Axis Review:** Evaluate every change for Correctness, Readability, Architecture, Security, and Performance.
- **Zero Secrets:** Never hardcode or commit API keys, passwords, or sensitive credentials. Use `.env` files (excluded from git) or secret managers.
- **Input Validation:** Treat all data from external sources (Zenoh, sockets, files, guest MMIO) as untrusted. Validate at the system boundary (e.g., check MMIO write sizes, validate YAML schemas).
- **No N+1 Patterns:** Ensure data fetching (if any) uses batching or joins. Avoid unbounded loops over external resources.

### 5. Shipping & Reliability
- **Rollback Readiness:** Every deployment-impacting change should consider how it can be reverted.
- **Observability:** Ensure critical paths (sim loop, clock sync) have appropriate logging (not in hot loop), error reporting, and health checks.
- **Documentation:** Update READMEs, ADRs, and API docs as the architecture evolves.

### 6. Protected Files & Centralized Scripts
- **DO NOT** automatically edit `.env` or `BUILD_DEPS` files. These files contain specific version pins and local secrets (via symlinking) that should only be modified by the user or dedicated synchronization scripts (e.g., `make sync-versions`).
- **QEMU Patching:** All modifications to QEMU source files, C-level hooks, or SysBus definitions MUST be centralized in `scripts/apply-qemu-patches.sh`. Do not duplicate `sed` commands or `git am` calls across Dockerfiles or bare-metal setup scripts.
- **Release Binary Portability (DEBIAN_CODENAME rule):** The `DEBIAN_CODENAME` in `BUILD_DEPS` is the single source of truth for the Debian base image used in **all** stages (devcontainer, CI, release builds). Do NOT hardcode a different codename in workflow `build-args` to paper over ABI incompatibilities. Instead, disable the problematic optional QEMU feature at configure time (e.g., `--disable-dbus-display`) so the binary remains compatible with the target platform without downgrading the entire build environment. Deviating DEBIAN_CODENAME between devcontainer and release violates environment parity (§7) and creates "works here, broken in release" bugs.

### 7. Environment Parity (1:1 Local-to-Remote Sync)

There are three escalating local gates; each is a strict superset of the previous:

| Gate | When to run | What it does |
|---|---|---|
| **Hooks** (`make lint && make test-unit`) | Every commit/push automatically | Runs directly in the devcontainer — no Docker spawn. Fast (~3-5 min). |
| **`make ci-local`** | Before opening a PR | Builds `devenv-base` locally and runs the identical three `docker run` steps that `.github/workflows/ci-main.yml` executes: `lint → build-tools → test-unit`. |
| **`make ci-full`** | Before merging to main | `ci-local` + ASan + Miri + full `builder` Docker image + all smoke phases run sequentially. Authoritative "will GitHub be green?" answer. |

**Why hooks run directly (not inside Docker):** The devcontainer IS `devenv-base`. Running `make lint` directly in the devcontainer is identical toolchain coverage without the CARGO_HOME conflict that corrupted Cargo's fingerprint cache. Reserve `make ci-local` for the containerised simulation.

**The CARGO_HOME / cache-isolation mandate:** Every `docker run devenv-base` invocation in the Makefile MUST use the following two flags — never use `-e CARGO_HOME=...` with a host path:
```
-e CARGO_TARGET_DIR=/tmp/ci-target          # compiled artifacts stay inside container
-v ci-cargo-registry:/usr/local/cargo/registry  # named volume for downloaded crates
```
Sharing `target/` between the host (CARGO_HOME=/usr/local/cargo) and a container running a different CARGO_HOME corrupts Cargo fingerprints and causes "can't find crate" errors on proc-macro crates. The named volume replaces the old `.cargo-cache` host-path bind-mount. See `docs/CI_GUIDE.md` for the full mount table.

- **Reference:** For details on the CI architecture, mount strategy, and the `devenv-base` vs `devenv` split, consult `docs/CI_GUIDE.md`. Whenever modifying CI pipelines or `Makefile` targets, you must maintain this strict 1:1 synchronization constraint.

### 8. Enterprise-Ready Quality (No Quality Regression)
- **Mandate:** Agents are NEVER allowed to lower the quality, strictness, or coverage of lints, static analyzers, type-checkers, or security gates without explicit human written consent.
- **YOLO Mode Constraint:** Even when running in `--yolo` mode, agents can only *increase* software quality and enterprise-readiness on their own (e.g., by enabling stricter rules or fixing technical debt). They must never suppress warnings, disable lints (`#[allow(...)]`, `noqa`, etc.), or bypass the type system to resolve errors unless specifically instructed to do so for a verified edge case.

### 9. No Polling / Sleep Avoidance (Determinism)
- **BANNED:** Using `std::thread::sleep` or `time.sleep()` in simulation hot paths, MMIO handling, network callbacks, or testing scripts (unless testing specific wall-clock boundaries).
- **CI Enforcement:** `make lint-rust` includes a grep gate: `grep -r "thread::sleep" hw/rust/ --include="*.rs"` must find zero matches. Any exception requires an explicit `// SLEEP_EXCEPTION: <reason>` comment and grep allowlist update.
- **REQUIRED (Rust):** Use event-driven synchronization (Condition Variables or `crossbeam_channel`) instead of polling loops. For background threads that must wait on a shutdown signal, use `condvar.wait_timeout()` keyed on a `shutdown: Arc<AtomicBool>` — this wakes immediately when shutdown is set rather than waiting the full sleep interval.
- **REQUIRED (Python Tests):** Use the `TimeAuthority` to advance virtual time deterministically, or rely on Zenoh Pub/Sub events for signaling, instead of arbitrary `time.sleep()` delays. Flaky tests are often caused by wall-clock assumptions in a virtual-time environment.

### 10. Safe Big QEMU Lock (BQL) Usage
The BQL is the most critical bottleneck and deadlock risk in the emulator.
- **Async Threads (Network, UART):** Background threads (like Zenoh subscribers) MUST NEVER block waiting to acquire the BQL. Instead, they must push data to lock-free queues (e.g., `crossbeam_channel::unbounded`) and let a QEMU timer (which already holds the BQL) process the queue.
- **Sync Threads (MMIO):** When a vCPU thread MUST block (e.g., waiting for an external bridge response), it must safely yield the BQL to prevent starving QEMU.
- **Bql API (Rust):**
  - Use `virtmcu_qom::sync::Bql::lock()` for most cases (returns RAII guard).
  - Use `Bql::lock_forget()` for explicit ownership transfer to C.
  - Use `Bql::temporary_unlock()` (returns `Option<BqlUnlockGuard>`) to safely yield the lock when blocking. It is safe to call even if the lock is not held.
  - Use `QemuCond::wait_yielding_bql(&mut mutex_guard, timeout_ms)` when a vCPU thread must wait on a CondVar — this is the only approved pattern for blocking an MMIO thread. Do not hand-roll the BQL-release / condvar-wait / BQL-reacquire sequence manually.
- **BANNED:** Manually calling `virtmcu_bql_unlock()`, `virtmcu_bql_lock()`, `virtmcu_mutex_lock()`, or `virtmcu_mutex_unlock()` from peripheral code without a guard or `lock_forget()`. All direct FFI calls to these functions are banned outside of `virtmcu-qom/src/sync.rs`.
- **BANNED:** Using `std::mem::forget(Bql::lock())` — use `Bql::lock_forget()` instead.
- **BANNED:** Mixing `std::sync::Mutex` and raw `*mut QemuMutex` for the same conceptual state in one device. Pick one locking scheme per device and document it.
- **REQUIRED:** Use safe, encapsulated abstractions provided in `virtmcu-qom/src/sync.rs` (like yielding CondVar wrappers) to ensure the BQL is always dropped and re-acquired in the exact correct order, even in the event of thread panics.
- **Lock Ordering (canonical):** BQL → peripheral mutex → (condvar wait releases peripheral mutex temporarily). Document this order in a module-level comment in every new peripheral that uses locking.

### 11. New Peripherals and the Educational C Model
- **Rust First**: All new peripheral models MUST be written in Rust using the `hw/rust/rust-dummy` template, unless they are specifically being ported from SystemC/C++.
- **Educational C Model**: We maintain one legacy C peripheral model (`hw/misc/educational-dummy.c`, instantiated as `dummy-device`) strictly for educational purposes and backward compatibility testing. It is automatically tested in Phase 2 integration tests to ensure we don't lose the ability to load traditional C plugins.

### 12. Safe Peripheral Teardown
Every peripheral that spawns background threads or blocks vCPU threads MUST implement a clean shutdown sequence:
1. Set `running = false` in shared state (holding the state lock).
2. Signal all condvars (`resp_cond.broadcast()`, `connected_cond.broadcast()`) so blocked threads wake up and check the running flag.
3. Wait for all active vCPU threads to exit (via `drain_cond` signaled when `active_vcpu_count` reaches zero) — **never use a bounded spinloop**.
4. Join the background thread.
5. Only then drop `Arc<SharedState>` (which frees QEMU mutex/condvar memory).
- **BANNED:** Bounded spinloops (`while count > 0 && attempts < N { yield_now() }`) as teardown drain mechanisms. They create a time-bomb UAF when the bound is exhausted.
- **Verification:** Every new peripheral must have a shutdown integration test that triggers teardown while a vCPU thread is blocked in an MMIO operation and asserts clean exit with no sanitizer errors.

### 13. Unsafe Rust — Precise Rules
- **Packed struct fields**: Never dereference a `*const T` where `T` is `#[repr(packed)]` directly. Always use `ptr::read_unaligned`. Packed structs read from byte buffers (`&[u8]`) must always go through `read_unaligned` to avoid UB on architectures that require alignment.
- **`transmute` for serialization**: Prefer explicit byte-order field serialization (e.g., `byteorder` crate or manual `to_le_bytes()`/`from_le_bytes()`) over `mem::transmute` on structs. `transmute` silently breaks if struct layout has padding.
- **`to_ne_bytes()` / `from_ne_bytes()` are BANNED for wire protocols**: `to_ne_bytes()` and `from_ne_bytes()` must NEVER be used when serializing values that cross a process or machine boundary (socket, Zenoh, shared memory). They silently corrupt data on big-endian hosts and create cross-platform incompatibility. Always use `to_le_bytes()` or `to_be_bytes()` with an explicit comment stating the expected wire byte order. `to_ne_bytes()` is permitted only for intra-process data structures that never leave the current process. **CI enforcement:** `grep -rn "to_ne_bytes\|from_ne_bytes" hw/rust/ --include="*.rs"` must find zero matches. Any exception requires a `// NE_BYTES_EXCEPTION: <reason>` comment.
- **`unsafe impl Send/Sync`**: Every `unsafe impl Send` or `unsafe impl Sync` must have a comment immediately above it explaining the invariant that makes it safe (e.g., "Safe: raw pointer is only accessed while BQL is held").
- **Minimize `unsafe` scope**: Keep `unsafe` blocks as small as possible — ideally one FFI call per block. Do not aggregate multiple unsafe operations in a single block.
- **No raw pointer byte copies for deserialization**: Never use `ptr::copy_nonoverlapping` to copy bytes into a `&mut T` for the purpose of deserializing a wire value. This is equivalent to `mem::transmute` and has the same endianness and padding risks. Use `T::from_le_bytes()` / `T::from_be_bytes()` (for primitives) or the struct's `unpack()` method instead.

### 14. Test Quality Mandates
- **Mock fidelity**: Test mocks in `sync.rs` (and elsewhere) must accurately simulate the invariant they replace. A mock that always returns "success" makes timeout and error paths invisible to the test suite — this provides false confidence. Mocks must support configurable return values and state.
- **Concurrency tests**: For any code that involves two or more threads with shared state (CondVar patterns, atomic state machines), write a `loom`-based test OR at minimum a stress test that runs the concurrent path 10 000+ times under `cargo test --release`. Use `loom` when the state space is small enough (2-3 threads, few operations); use stress tests otherwise.
- **Teardown tests**: Any peripheral that spawns threads must have a test that verifies clean shutdown. Run under Miri (`cargo miri test`) for the Rust unit test suite to catch UB in teardown paths.

### 15. Lessons Learned & Architectural Gotchas
To prevent historical regressions, agents and developers must strictly avoid these known anti-patterns derived from past incident postmortems (e.g., Phase 7 & 18.9):
- **The "DSO TLS Trap":** QEMU macros or functions relying on file-static or coroutine-local TLS (like `bql_locked()`) **cannot** be reliably called from a plugin/DSO. The linker resolves the symbol, but the DSO reads its own uninitialized local copy of the TLS slot. **Rule:** Always use main-binary wrapper functions (e.g., `virtmcu_is_bql_locked()`) exported from a proper header.
- **Multi-Flag Protocol Hygiene:** In producer-consumer synchronization (e.g., `quantum_ready` / `quantum_done`), every signal flag **must be reset by the consumer** immediately after consumption. Never leave a "done" flag set, as it causes stale reads and deadlocks in the next cycle.
- **Zenoh Executor Deadlocks:** Never block a Zenoh async executor thread with synchronous calls like `.wait()`, `.recv()`, or `thread::sleep()` inside a callback. **Rule:** Offload blocking work to a background thread and use non-blocking channels (`crossbeam_channel::unbounded` or `bounded`) to bridge Zenoh and QEMU.
- **UART / Hardware FIFO Backpressure:** QEMU's UART models (like PL011) have fixed-size hardware FIFOs (e.g., 32 bytes). Shoving larger packets via Zenoh without checking capacity causes silent data drop. **Rule:** Peripherals must implement backpressure. Check `qemu_chr_be_can_write`, buffer leftovers in a backlog queue, and use the `chr_accept_input` hook to drain the backlog when the guest FIFO frees up.
- **Testing Failure States:** Smoke tests verifying the "happy path" are insufficient. **Rule:** Implement "Hammer" (high frequency), "Flood" (high volume), and "Stall" (e.g., sending massive 10-billion-instruction quanta to trigger wall-clock timeouts) tests to explicitly verify error recovery and backpressure paths.
- **Automated Patching Mandate:** Manual local edits to `third_party/qemu` cause "Works on my machine" bugs because CI clones a fresh QEMU tree. **Rule:** All modifications to untracked third-party code MUST be implemented via the automated injection scripts (`scripts/apply-qemu-patches.sh` or `apply_zenoh_hook.py`).
- **No One-Shot Patch Scripts in Repo Root:** Agents MUST NOT commit one-shot Python or shell scripts (`patch_*.py`, `fix_*.py`, `pathlib_*.py`, etc.) to the repository root. These are development scaffolding that belongs only in a git-ignored scratch directory or should be deleted immediately after use. Committing them pollutes the repo, fails `make lint`, and misleads reviewers. If a bulk transformation is needed, implement it as a permanent, tested utility in `scripts/` — or delete it before committing.


---

## Common Pitfalls & Troubleshooting

### Parallel Test Interference
Since tests run in parallel, "random" failures are often caused by resource contention (e.g., two tests trying to bind to the same Zenoh port or UNIX socket).
- **The Fix**: Use dynamic port allocation or unique identifiers for socket paths. Check if a failure is reproducible when running the test in isolation (`pytest tests/test_foo.py`) vs. the full suite.

### Stale Simulation Processes (Multiple QEMU Instances)
During development, you may encounter "stale" or orphaned QEMU or Zenoh processes. These processes can hold onto ports, UNIX sockets, or CPU resources, causing subsequent runs to fail with "Address already in use" or mysterious timeouts.

**The Fix:** Manually terminate any lingering processes or use the project cleanup script:
```bash
make clean-sim
# or
bash scripts/cleanup-sim.sh
```
*Note: Stale process interference is the #1 cause of "it passes locally but fails next time" bugs.*

### Interactive QEMU Debugging
If a node hangs or fails to boot, run it interactively to see the firmware's console output.
- **Action**: Run the `run.sh` command without `-monitor none` or `-serial file:...` and use `-nographic`.
- **Exit**: To exit an interactive QEMU session, press `Ctrl+A` followed by `X`.

### SysBus Mapping vs. `-device`
In the `arm-generic-fdt` machine, a device added via `-device` is NOT automatically mapped into guest memory. 
- **The Cause**: Mapping only occurs if a corresponding node exists in the Device Tree (DTB) with a `reg` property.
- **The Fix**: Always declare peripherals in the board YAML files. The `yaml2qemu.py` tool handles the mapping.
- **QOM Naming Convention**: Top-level peripherals defined in YAML are mapped to the QOM tree purely by their name, *without* the `@<address>` suffix (e.g., `/flexray` instead of `/flexray@9003000`). Use `qom-list /` to verify paths.

### MMIO Offset Contract & Endianness
The `mmio-socket-bridge` delivers **region-relative offsets**, not absolute physical addresses. 
- **Example**: If a bridge is at `0x10000000` and the guest reads `0x10000004`, the bridge receives `0x04`.
- **Requirement**: Your MMIO adapter/model must handle these offsets directly without adding the base address.
- **Endianness**: All ARM-based virtual machines in this project operate in **Little Endian**. When verifying raw Zenoh payloads or memory dumps, a 32-bit register write of `0xDEADC0DE` by the firmware will be serialized over the wire as `DE C0 AD DE`.

### "Works on My Machine" (Local vs. CI Drift)
If a fix passes locally but fails in CI, it's often due to manual edits in `third_party/qemu` or untracked files that aren't part of the automated patch mechanism.
- **The Rule**: If CI cannot reproduce your local setup from `git clone` + `make setup`, your fix is not complete.
- **The Check**: Always run `git status` after a debugging session to ensure all changes are tracked.

---

## Before Every Commit — Mandatory Lint Gate

To ensure CI remains green and Rust code follows project standards, you MUST run the linting suite before every commit:

```bash
make lint     # Runs ruff, version checks, cargo clippy -D warnings, sleep-ban grep, and more
```

**`cargo clippy` is run with `-D warnings`** — every warning is a build failure. This means:
- `#[allow(clippy::...)]` suppressors in production code are banned (they would need to suppress a warning that is now an error).
- `#[allow(static_mut_refs)]` is banned — fix the underlying `static mut` pattern instead.
- `#[allow(clippy::too_many_lines)]` is banned — split the function instead.

### Git Hooks (Automation)
Both the `pre-commit` and `pre-push` hooks run `make lint && make test-unit` **directly** in the current shell (devcontainer or native Mac environment). There is no nested `docker run` — the devcontainer already is `devenv-base`.

- **Installation**:
  - **Devcontainer**: Hooks are installed automatically on container creation.
  - **Manual**: Run `make install-hooks` to install or reinstall them.
- **Bypassing**: If you must skip for a work-in-progress commit you know is messy:
  ```bash
  git commit -m "wip: messy change" --no-verify
  git push --no-verify
  ```

**What the hooks run:** `make lint` (ruff, clippy -D warnings, check-ffi, shellcheck, …) followed by `make test-unit` (Rust + Python unit tests, no QEMU). Total wall time: ~3-5 min.

**For full CI parity before a PR:** run `make ci-local`. For complete pre-merge validation: run `make ci-full`.

---

## CI/CD Troubleshooting & "Make CI Green" Workflow (Enterprise CI Fixer Loop)

When instructed to **"fix CI"**, **"make CI green"**, or address pipeline failures, you MUST enter the **Enterprise CI Fixer Loop**. This is not just about pushing a fix; it is about absolute empirical certainty.

1. **Diagnose Remotely:** Use the GitHub CLI (`gh run list`, `gh run view --log`) to identify the exact failure.
2. **Local Reproduction (The `ci-full` Gate):** You MUST reproduce the failure locally using `make ci-full` or the specific smoke phase directly: `docker run --rm -v $(pwd):/workspace -w /workspace -e USER=vscode $(BUILDER_IMG) bash scripts/ci-phase.sh <PHASE>`. If it passes locally but fails in CI, **STOP.** Align scripts, timeouts (`VIRTMCU_STALL_TIMEOUT_MS`), or environment until the failure is caught locally.
3. **Stress Test the Bug:** Once reproduced, run the failing test 100+ times. Quantify the failure rate.
4. **Exhaustive Implementation:** Implement the fix.
5. **Verified Recovery:** Prove the fix works by running the same stress test again. It must reach a 100% success rate over a significant number of runs.
6. **Final Lint Gate:** Run `make ci-local` to ensure that lints, build-tools, and unit tests pass in the containerised environment.
7. **Commit & Push:** Commit the fix and the new/updated stress tests.
8. **Monitor & Loop:** Autonomously monitor the new CI run (`gh run watch`). If it fails, restart this loop immediately. Do not stop until the pipeline is officially green.

---

## Local CI Loop Workflow (The "Local Green" Loop)

When asked to **"Fix CI locally"**, **"Run CI loop"**, or **"Make it pass"**, you MUST enter this autonomous loop:

1. **Identify Pipeline:** Determine the full suite of local validation commands (e.g., `make lint`, `make build`, `make test`).
2. **Execute & Diagnose:** Run the entire suite. Capture all failures (lint errors, build breaks, test regressions).
3. **Surgical Fix:** Address the first failure.
4. **Commit:** Once a fix is verified locally, commit the changes with a descriptive message (**DO NOT push**).
5. **Iterate:** Repeat steps 2-4 until the *entire* suite passes.
6. **Final Report:** Once all checks are green, report the major findings and fixes. Do not stop until the pipeline is fully passing.

---

## Test Coverage Loop Workflow (The "Coverage" Loop)

When asked to **"Increase coverage"** or **"Improve test coverage"**, you MUST enter this autonomous loop:

1. **Baseline:** Run the coverage tool (e.g., `make coverage` or `pytest --cov`) to identify current gaps.
2. **Targeting:** Identify the most critical untested paths (e.g., error handling, boundary conditions).
3. **Implement Tests:** Write new tests specifically targeting the identified gaps.
4. **Verify:** Run coverage again to confirm the increase. Ensure no existing tests were broken.
5. **Commit:** Commit the new tests and any necessary code changes.
6. **Iterate:** Repeat until the coverage goal is met or there are no more obvious improvements to be made.

---

## Note to Developers: Invoking Autonomous Loops

To trigger these workflows, use direct commands:
- *"Fix CI"* (Invokes the full Enterprise CI Fixer Loop: Reproduce -> Stress -> Fix -> Verify -> Push)
- *"Fix CI locally and commit."*
- *"Run the local CI loop until everything passes."*
- *"Increase code coverage for the physics module."*
- *"Keep fixing CI locally, don't stop until all lints and tests pass."*
