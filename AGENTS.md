# AGENTS.md — virtmcu Project Context

> [!IMPORTANT]
> **ENTERPRISE QUALITY MANDATE**: All agents (Codex, Gemini, etc.) must produce only enterprise-grade code. No "AI-style" shortcuts, no suppressed lints (`#[allow]`, `noqa`, `// @ts-ignore`), and no bypassing the type system. Every change must be surgically precise, idiomatically perfect, and backed by empirical test evidence.

Read automatically by Codex and Gemini CLI at session start (`GEMINI.md` is a symlink — maintain only this one). Update when architectural decisions change or new constraints are discovered.

---

## TOP PRIORITY: Binary Fidelity

**The same firmware ELF that runs on a real MCU must run unmodified in VirtMCU.**

- No virtmcu-specific startup code, linker sections, or compile-time flags in firmware.
- Peripherals mapped at the **exact** base addresses the real MCU datasheet specifies.
- Register layouts, reset values, and interrupt numbers must match physical silicon.
- `clock` and all co-simulation infrastructure are **invisible to the firmware** — QEMU-level only, no guest MMIO exposure.
- Any feature requiring firmware modification is a VirtMCU bug. See [ADR-006](docs/architecture/adr/ADR-006-binary-fidelity.md).

---

## SECOND PRIORITY: Global Simulation Determinism

**Same topology YAML + same firmware ELFs + same `global_seed` → bit-identical output on every run.**

- **Topology declared, not discovered**: full network graph in world YAML, loaded by `DeterministicCoordinator` at startup. Runtime Zenoh peer-mode scouting is BANNED.
- **Zenoh Session Isolation**: All Zenoh sessions in Python tests MUST use `make_client_config()` or the `zenoh_session` fixture. Raw `zenoh.open()` is BANNED to prevent parallel test cross-talk (scouting=false, mode=client).
- **Canonical tie-breaking**: same-vtime messages delivered in order `(delivery_vtime_ns, source_node_id, sequence_number)` by the coordinator — never by OS scheduling.
- **Per-quantum barrier**: coordinator withholds quantum-Q messages until ALL nodes signal "quantum Q complete" (PDES barrier pattern).
- **Automated Synchronization (SOTA)**: The framework implicitly injects the `-S` flag to launch QEMU frozen, handles routing synchronization internally (`ensure_session_routing`), and issues `cont` via QMP. Do not manually call `ensure_session_routing` in tests.
- **Stochastic seeding**: derive per-node PRNG as `seed_for_quantum(global_seed, node_id, quantum_number)`. `rand::thread_rng()` and wall-clock seeding are BANNED.
- **Mobile nodes**: topology changes pushed by physics engine before each quantum step, never discovered at runtime.
- Any feature producing different output across identical runs is a VirtMCU bug. See [ADR-014](docs/architecture/09-determinism-and-chaos.md).

---

## What This Project Is

**virtmcu** is a **deterministic multi-node firmware simulation framework** built on QEMU:
1. **Dynamic QOM device plugins** (.so shared libraries).
2. **arm-generic-fdt machine** — ARM machines defined by Device Tree.
3. **Native VirtMCU QOM plugin** (`hw/rust/`) — deterministic clock and I/O.
4. **yaml2qemu** — Parses OpenUSD-aligned YAML and emits `.dtb` + CLI.

---

## Clock Synchronization Model

| Mode | How to invoke | When to use |
|---|---|---|
| `standalone` | No `-device virtmcu-clock` | Rapid development, logic testing. |
| `slaved-suspend` | `-device virtmcu-clock,mode=slaved-suspend` | **Default.** Deterministic co-simulation. |
| `slaved-icount` | Same + `-icount shift=0,align=off,sleep=off` | Sub-quantum timing precision (PWM, µs). |

### Error Codes (`sim/clock/advance/{id}` reply)
- `0` OK: quantum completed.
- `1` STALL: QEMU did not reach TB boundary within stall timeout. QEMU **stays alive** and logs "STALL DETECTED" — the Python orchestrator must terminate it if recovery is not desired.
- `2` ZENOH_ERROR: transport layer failure.

### Stall-Timeout Contract
- **Never hardcode `stall-timeout=<ms>` in test `extra_args`** unless validating stall detection explicitly.
- **Timeout Multiplier**: Timeouts dynamically scale based on the environment (e.g., 5.0x under ASan). The `qemu_launcher` automatically injects the appropriately scaled `stall-timeout` via `conftest_core.py::get_time_multiplier()`.
- **Logical Timeouts**: In test code, pass ideal *logical* timeouts to `vta.step(timeout=10.0)` or `bridge.wait_for_line(timeout=10.0)`. The framework will mathematically stretch these into real-world bounds transparently. Do NOT use massive arbitrary timeouts like `timeout=500` to fix slow CI runners.

---

## Timing Model and Constraints

### 1. MMIO Socket Blocking
`mmio-socket-bridge` blocks the QEMU TCG thread per MMIO op. Virtual time does NOT advance while blocked. High latency → clock stalls.

### 2. FFI Layout Verification ("FFI Gate")
- All core QOM structs in Rust MUST have `assert!` checks for `size_of` and `offset_of`.
- Before modifying a Rust FFI struct, verify ground truth: `./scripts/check-ffi.py` (run `make build` first if QEMU binary is stale).
- Auto-sync: `./scripts/check-ffi.py --fix`.

### 3. Plugin Staleness Prevention
`scripts/run.sh` finds the freshest `.so` automatically. Do NOT override `QEMU_MODULE_DIR`.

### 4. Simulation Hygiene & Parallel Safety
- NEVER hardcode ports (e.g., 7447) or socket paths. Use `scripts/get-free-port.py` or `tempfile.mkdtemp()`.
- `pytest -n auto` is standard. All fixtures must be isolated.
- `scripts/cleanup-sim.sh` is workspace-scoped (inspects `/proc/<pid>/cwd`) — safe alongside other agents.

### 5. WFI Behavior
In `slaved-suspend`, virtual time advances during WFI. Quantum boundaries still trigger clock-halt. Prefer ARM Generic Timer at 100 Hz over tight polling.

---

## Key Constraints

- **MMIO Delivery**: `mmio-socket-bridge` delivers **relative offsets**. Do NOT add the base address.
- **DTB Validation**: `yaml2qemu` validates all YAML peripherals are in the DTB — missing entries fail the build.
- **SysBus Mapping**: `-device`-only devices are NOT mapped into guest memory → Data Abort. Declare in YAML.
- **Topology-First**: full graph in `topology:` YAML before start. Coordinator rejects unlisted connections (logged as violations). Topology changes pushed by physics engine, not discovered.
- **Clock/Comms Separation**: clock sync (`ClockSyncTransport`) and emulated network (`DeterministicCoordinator`) use separate transports. Never mix.

---

## Development Efficiency

- **`make setup`**: initial setup or QEMU dependency changes only — triggers massive rebuilds.
- **`make build`**: standard incremental builds for `hw/` peripherals.
- **Targeted ninja**: `ninja -C third_party/qemu/build-virtmcu$( [ "$VIRTMCU_USE_ASAN" = "1" ] && echo "-asan" ) hw-virtmcu-<name>.so`
- **Pre-installed QEMU**: available at `/opt/virtmcu`; rebuild only when modifying the emulator or plugins.

---

## Directory Structure

```
virtmcu/
├── hw/                         # Hardware peripheral models
│   └── rust/                  # Rust-based QOM plugins
│       ├── backbone/          # Clock, MMIO bridge, Transport abstractions
│       ├── comms/             # Netdev, CAN, SPI, UART, WiFi
│       ├── observability/      # Actuator, Telemetry, UI
│       ├── mcu/               # MCU-specific peripherals
│       └── common/            # Shared APIs and QOM helpers
├── tools/
│   └── yaml2qemu.py            # YAML -> DTB transpiler with validation
└── docs/
    ├── architecture/           # The Virtmcu Specification: core design, temporal sync, PDES, ADRs
    ├── guide/                  # User & developer guides (build system, containers, CI)
    └── postmortem/             # Historical critiques and CI issue RCAs
```

## Dependency & Version Control

- **Centralized Versions**: adhere to `BUILD_DEPS` for QEMU, Zenoh, and core deps. Verify before upgrades.
- **Package Management**: use `uv` (`uv pip`, `uv run`) over `pip` for all Python tooling.

---

## Language Selection Policy (ADR-013)

| Component | Language | Rule |
| :--- | :--- | :--- |
| **Sim Loop** | **Rust** | **NATIVE ONLY.** All core plugins migrated to Rust. |
| **Physics/SystemC** | **C++** | Standard for TLM-2.0 / MuJoCo. |
| **Tooling/Parsing** | **Python** | Out-of-band only. |
| **Telemetry** | **Rust** | Direct FlatBuffers/Zenoh integration. |

**Banned:** Python in the hot simulation loop (MMIO/Clock/Netdev).

---

## Bifurcated Testing Strategy (Rust + Python)

| Layer | Language | Framework | Purpose |
| :--- | :--- | :--- | :--- |
| **White-Box Internals** | **Rust** | `cargo test` | State machines, layout validation, protocol parsing, FFI boundaries. |
| **Black-Box Orchestration** | **Python** | `pytest` + `asyncio` | Multi-process orchestration, QMP, UART verification, end-to-end regression. Uses `SimulationTransport` for transport agnosticism. |
| **Thin CI Wrappers** | **Bash** | `make test` / `.sh` | 2-3 line entry points only — invoke `pytest` or `cargo test`. |

### Parallel Execution Rules (`pytest -n auto` — all tests MUST comply)

1. **NO Hardcoded Ports**: BANNED: fixed ports (`7447`, `7450`) anywhere. REQUIRED: `zenoh_router` fixture or `scripts/get-free-port.py`.
2. **NO Hardcoded Paths**: BANNED: shared temp paths (e.g., `/tmp/yaml_boot/out.dtb`). REQUIRED: `pytest` `tmp_path` fixture or `mktemp -d`.
3. **NO Random Collisions**: BANNED: `random.randint()` / generic UUIDs for node IDs. REQUIRED: deterministic uniqueness via `os.getpid()`, `worker_id`, or `tmp_path`.
4. **NO Manual Process Management**: BANNED: spawning daemons (e.g., `deterministic_coordinator`) in test bodies. REQUIRED: centralized `pytest` fixtures with automated teardown.
5. **Test Scope**: `pytest` scoped to `tests/` via `pyproject.toml`. Do NOT place test files in `tests/fixtures/guest_apps/<domain>/`.
6. **Binary Resolution**: check both `target/release/` and `tools/<tool_name>/target/release/` for Rust tool binaries.

**Mandates**: complex orchestration (QEMU + background process) → Python `pytest` fixture only. Internal logic → `#[test]` in Rust, no QEMU boot.

### Pro-Tips
- **`-S` Boot Freeze**: pass `-S` to QEMU; call `await bridge.start_emulation()` only after UART subscribers are set up.
- **Dynamic Config Injection**: `shutil.copy` to `tmp_path`, then `sed`/`replace()` to inject `zenoh_router` endpoint.
- **Bash Port Fallback**: `PORT=${1:-$(python3 "$WORKSPACE_DIR/scripts/get-free-port.py")}`
- **Time Authorities**: `VirtualTimeAuthority` for multi-node; legacy `TimeAuthority` for single-node.
- **Selective Cleanup**: `scripts/cleanup-sim.sh --filter <pattern>`.

---

## Production Engineering Mandates

### 1. Environment Agnosticism
- No absolute paths or user-specific home directory references. Use relative paths from project root.
- Use `os.path.join` (Python), `path::PathBuf` (Rust), `std::filesystem` (C++).
- All agents run in a **devcontainer** — `localhost` is the container. Never assume host toolchain access.

### 2. Explicit Constants (No Magic Numbers)
BANNED: inline literals. REQUIRED: named `const` with a comment explaining value and purpose.
```rust
/// Standard payload size for the bridge
const MAX_PAYLOAD_SIZE: usize = 1024;
let buffer = [0; MAX_PAYLOAD_SIZE];
```

### 3. Verification & TDD ("Beyonce Rule")
- Write a failing test reproducing the bug **before** implementing the fix.
- Every feature/change needs corresponding unit or integration tests.
- One logical change per commit/PR. Refactoring and behavior changes in separate commits.

### 4. Quality & Security Gates
- Review every change for Correctness, Readability, Architecture, Security, and Performance.
- No hardcoded secrets — use `.env` (gitignored) or secret managers.
- Validate all external inputs (Zenoh, sockets, files, guest MMIO) at system boundaries.

### 5. Shipping & Reliability
Every deployment change must be revertable. Add logging on critical paths (not in hot loop). Update ADRs/READMEs as architecture evolves.

### 6. Protected Files & Centralized Scripts
- **DO NOT** edit `.env` or `BUILD_DEPS` automatically — user-only or via `make sync-versions`.
- All QEMU source modifications → `scripts/apply-qemu-patches.sh`. No ad-hoc `sed`/`git am` elsewhere.
- **DEBIAN_CODENAME** in `BUILD_DEPS` is the single source of truth for all stages. Never deviate between devcontainer and release — disable incompatible QEMU features at configure time instead.
- **Python Testing Framework**: `pytest-asyncio` is pinned to **1.3.0** (with `asyncio_default_fixture_loop_scope = "function"` in `pyproject.toml`) to ensure compatibility with Python 3.13 and solve `FixtureDef` attribute errors. Agents MUST NOT change this version.

### 7. Environment Parity (1:1 Local-to-Remote)

| Gate | When | What |
|---|---|---|
| **Hooks** (`make lint && make test-unit`) | Every commit (auto) | Direct in devcontainer — fast (~3-5 min). |
| **`make ci-local`** | Before PR | Three `docker run` steps matching `.github/workflows/ci-main.yml`. |
| **`make ci-full`** | Before merge | `ci-local` + ASan + Miri + all integration domains. |

**CARGO_HOME isolation** — every `docker run devenv-base` MUST use:
```
-e CARGO_TARGET_DIR=/tmp/ci-target
-v ci-cargo-registry:/usr/local/cargo/registry
```
Sharing `target/` between host and container corrupts Cargo fingerprints. See `docs/guide/04-continuous-integration.md`.

### 8. Enterprise-Ready Quality (No Regression)
- Agents MUST NOT lower lint strictness, coverage, or security gates without explicit written human consent.
- In `--yolo` mode: only *increase* quality. Never suppress warnings (`#[allow(...)]`, `noqa`) or bypass the type system.

### 9. Logging Strictness (No Print Statements)
- BANNED: `print()` in Python (outside of explicit CLI tools excluded in `pyproject.toml`) and `println!`/`eprintln!` in Rust.
- REQUIRED: Use structured logging (`logger.debug`, `logger.info`, etc. via the `logging` module in Python) and VirtMCU simulation log macros (`sim_info!`, `sim_err!`, etc. from `virtmcu_qom` in Rust).
- CI enforcement: The `T201` Ruff rule enforces no `print()` statements in Python.

### 10. Protocol Serialization (No Manual Struct Packing)
- BANNED: Manual `struct.pack()`, `struct.unpack()`, or `struct.unpack_from()` for core simulation protocols (`MmioReq`, `ClockAdvanceReq`, `ZenohFrameHeader`, etc.).
- REQUIRED: Use `vproto.py` (which uses FlatBuffers) for all core protocol serialization and deserialization.
- CI enforcement: `Makefile` `lint-python` target greps for forbidden `struct` calls.

### 11. No Polling / Sleep Avoidance
- BANNED: `std::thread::sleep`, `time.sleep()`, or `asyncio.sleep()` in hot paths, MMIO, network callbacks, or tests. All test synchronization MUST be deterministic (e.g., using `vta.step()`, QMP events, or Zenoh `recv_async()`).
- CI enforcement: `grep -r "thread::sleep" hw/rust/` and `grep -r "(asyncio|time).sleep(" tests/` must be zero. Exception: `# SLEEP_EXCEPTION: <reason>`.
- Use `bridge.wait_for_line(..., timeout=...)` or `bridge.wait_for_event(...)` which leverage `asyncio.Event` signaling rather than busy-wait spinloops.
- Rust: use `condvar.wait_timeout()` keyed on `shutdown: Arc<AtomicBool>`.
- Python tests: use `TimeAuthority` for virtual time; Zenoh Pub/Sub for signaling.

### 12. Safe Big QEMU Lock (BQL) Usage

- **Async threads** (Transport subscribers): MUST NOT block waiting for BQL. Push to `crossbeam_channel::unbounded`; a QEMU timer (holding BQL) drains the queue. `SafeSubscription` handles this pattern automatically.
- **MMIO vCPU threads**: yield BQL via `Bql::temporary_unlock()` when blocking.
- **Bql API**: `Bql::lock()` (RAII), `Bql::lock_forget()` (ownership transfer to C), `Bql::temporary_unlock()` (safe yield), `QemuCond::wait_yielding_bql(guard, timeout_ms)` (only approved vCPU-wait pattern).
- BANNED: raw `virtmcu_bql_unlock/lock()` or `virtmcu_mutex_lock/unlock()` outside `virtmcu-qom/src/sync.rs`; `std::mem::forget(Bql::lock())`; mixing `std::sync::Mutex` with `*mut QemuMutex` in one device.
- **Lock order (canonical)**: BQL → peripheral mutex → condvar wait. Document in module-level comment.

### 13. New Peripherals
- All new peripherals in Rust using `hw/rust/common/rust-dummy` template.
- One legacy C model (`hw/misc/educational-dummy.c`, `dummy-device`) kept for compatibility; tested in dynamic_plugin.

### 14. Safe Peripheral Teardown

Mandatory shutdown sequence:
1. Set `running = false` (holding state lock).
2. Broadcast all condvars so blocked threads wake and check `running`.
3. Wait via `drain_cond` until `active_vcpu_count == 0` — **no bounded spinloops**.
4. Join background thread.
5. Drop `Arc<SharedState>`.

- BANNED: bounded spinloops (`while count > 0 && attempts < N`) — time-bomb UAF when bound exhausted.
- **Drain pattern**: `drain_cond: Arc<(Mutex<()>, Condvar)>`; callback calls `notify_all()` after decrement; Drop: `while active_count > 0 { guard = cvar.wait(guard).unwrap(); }`.
- Every new peripheral needs a shutdown integration test (teardown during blocked MMIO, no sanitizer errors).

### 15. Unsafe Rust — Precise Rules
- **Packed structs**: use `ptr::read_unaligned` — never direct dereference of `*const T` where `T: repr(packed)`.
- **Serialization**: use `to_le_bytes()`/`from_le_bytes()` or per-field byte-order ops — not `mem::transmute`.
- **Wire protocols**: `to_ne_bytes()` / `from_ne_bytes()` BANNED for cross-process/machine values. CI: `grep -rn "to_ne_bytes\|from_ne_bytes" hw/rust/` must be zero. Exception: `// NE_BYTES_EXCEPTION: <reason>`.
- **`unsafe impl Send/Sync`**: comment above every instance explaining the safety invariant.
- **`unsafe` scope**: one FFI call per block — no aggregated unsafe ops.
- **Deserialization**: no `ptr::copy_nonoverlapping` into `&mut T`. Use `from_le/be_bytes()` or `unpack()`.

### 16. Test Quality Mandates
- **Mock fidelity**: mocks must support configurable return values — "always success" mocks hide error paths.
- **Concurrency**: `loom`-based test (small state space) or 10 000+ iteration stress test (`cargo test --release`).
- **Teardown**: every thread-spawning peripheral needs a clean-shutdown test; run under `cargo miri test`.

### 17. Python SOTA Mandates (Tooling & Testing)
- **No Path Bootstrapping**: BANNED: `sys.path.insert()`, `sys.path.append()`. Scripts MUST rely on `uv run` and the `pyproject.toml` package boundary.
- **No Global Path Mutation**: BANNED: `os.chdir()`. Use absolute `pathlib.Path` objects or pass `cwd=` to `subprocess`.
- **AST over Regex**: BANNED: using regex or string searches (`.find()`) to parse structured data like `.dtb`, JSON, or YAML. Use native parsers (e.g., the `fdt` library).
- **First-Class Tooling**: Scripts in `tools/` and `scripts/` are production code. They must pass strict type-checking (`mypy`) and cannot use `# noqa` or `# type: ignore` to bypass architecture rules.
- **No Hardcoded Binary Names**: BANNED: using literal strings like `"deterministic_coordinator"` in `get_rust_binary_path` or `resolve_rust_binary`. REQUIRED: Use `VirtmcuBinary.DETERMINISTIC_COORDINATOR` from `tools.testing.virtmcu_test_suite.constants`. CI enforces this via AST lints.


### 18. Lessons Learned (Anti-Patterns — Do Not Repeat)

- **SafeSubscription Teardown**: never bound a drain loop. Use `Condvar::notify_all()` in callback + unconditional `Condvar::wait()` in Drop (see §12). `SafeSubscription` encapsulates this logic.
- **PDES Tie-Breaking**: direct pub/sub between nodes is BANNED. All inter-node traffic routes through `DeterministicCoordinator` for canonical ordering.
- **DSO TLS Trap**: never call QEMU TLS macros (e.g., `bql_locked()`) from a plugin DSO — use `virtmcu_is_bql_locked()` from the main-binary header.
- **Atomic State Transitions**: use a single `AtomicU8` enum + `compare_exchange`. Multiple boolean flags allow illegal states.
- **Zenoh Executor Deadlocks**: never block a Zenoh async thread with `.wait()`, `.recv()`, or `thread::sleep()`. Offload to a background thread via `crossbeam_channel`.
- **UART FIFO Backpressure**: PL011 FIFO is 32 bytes. Check `qemu_chr_be_can_write`, buffer overflow in backlog, drain via `chr_accept_input`.
- **Test Failure Paths**: implement Hammer, Flood, and Stall tests — happy-path smoke tests are insufficient.
- **QEMU Patch Automation**: never hand-edit `third_party/qemu`. All changes via `scripts/apply-qemu-patches.sh` or `apply_zenoh_hook.py`.
- **Reference Material**: vendor SDK, firmware, and spec PDFs → `third_party/golden_references/<mcu_name>/` (gitignored). Each subfolder needs `README.md` with URL, license, date. Firmware binaries in `tests/firmware/` need `PROVENANCE.md`.
- **No One-Shot Scripts in Root**: `patch_*.py`, `fix_*.py` etc. belong in gitignored scratch dirs or must be deleted before commit. Permanent utilities go in `scripts/`.

---

## Common Pitfalls & Troubleshooting

- **Parallel test interference**: random failures → port/socket contention. Use dynamic allocation. Verify with `pytest tests/test_foo.py` in isolation.
- **Stale processes**: orphaned QEMU/Zenoh hold ports → `make clean-sim` or `bash scripts/cleanup-sim.sh`. (#1 cause of "passes once, fails next time".)
- **Interactive debugging**: run without `-monitor none`/`-serial file:...`, use `-nographic`. Exit: `Ctrl+A X`.
- **SysBus mapping**: `-device`-only → not in guest memory. Declare in YAML. QOM path has no `@<address>` suffix; verify with `qom-list /`.
- **MMIO offsets**: bridge delivers region-relative offsets, not absolute addresses. ARM is Little Endian (`0xDEADC0DE` on wire: `DE C0 AD DE`).
- **Local vs. CI drift**: hand-edits to `third_party/qemu` don't survive CI clone. Run `git status` after debugging; fix must be reproducible from `git clone` + `make setup`.

---

## Before Every Commit — Mandatory Lint Gate

```bash
make lint     # ruff, version checks, cargo clippy -D warnings, sleep-ban grep, and more
```

`[workspace.lints.clippy] all = "deny"` — every clippy warning is a build failure. `#[allow(clippy::...)]`, `#[allow(static_mut_refs)]`, and `#[allow(clippy::too_many_lines)]` are all BANNED in production code.

**Git hooks** (`pre-commit` + `pre-push`): run `make lint && make test-unit` directly in the devcontainer shell (~3-5 min). Install: `make install-hooks`. Skip (WIP only): `git commit --no-verify` / `git push --no-verify`.

**Full CI parity before PR:** `make ci-local`. Complete pre-merge validation: `make ci-full`.

---

## CI Workflows

### "Fix CI" / "Make CI Green" (Enterprise CI Fixer Loop)
1. Diagnose: `gh run list` / `gh run view --log`.
2. Reproduce locally: `make ci-full` or `docker run --rm -v $(pwd):/workspace -w /workspace -e USER=vscode $(BUILDER_IMG) bash scripts/ci-integration-tier.sh <DOMAIN>`. If local passes but CI fails — **STOP** and align environments first.
3. Stress-test the bug: 100+ runs, quantify failure rate.
4. Implement fix.
5. Stress-test again: must reach 100% success rate.
6. `make ci-local` (lint + build-tools + unit tests in container).
7. Commit fix + updated stress tests.
8. `gh run watch` — loop back to step 1 if it fails. Do not stop until green.

### "Fix CI locally" / "Run CI loop" / "Make it pass"
1. Identify full suite (`make lint`, `make build`, `make test`).
2. Run suite — capture all failures.
3. Fix the first failure.
4. Commit (**do NOT push**).
5. Repeat 2-4 until suite is fully green, then report.

### "Increase coverage" / "Improve test coverage"
1. Baseline: `make coverage` or `pytest --cov`.
2. Target most-critical untested paths (error handling, boundaries).
3. Write tests; re-run coverage to verify increase.
4. Commit; iterate until goal met.
