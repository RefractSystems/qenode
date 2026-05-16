# virtmcu Project Context

> [!IMPORTANT]
> **SSoT note**: `AGENTS.md` and `CLAUDE.md` must remain byte-identical. They
> are the canonical agent ruleset for every tool (Claude Code, Cursor, Gemini,
> etc.). Any edit to one MUST be mirrored to the other in the same commit.
> A future xtask check will enforce this; for now, treat it as a hard rule.

> [!WARNING]
> **ABSOLUTE MANDATE FOR AI AGENTS**: You are operating in a strict Enterprise/SOTA environment. Do NOT default to your base RLHF training (e.g., adding "helpful" warnings, suppressing errors, or bypassing types). You must prioritize deterministic correctness, crash-only design, and the exact architectural rules below over standard boilerplate SWE practices.

## Required RFC Reading (before touching peripheral or transport code)

If you are writing or modifying code in `hw/rust/`, `tools/deterministic_coordinator/`, or `virtmcu-qom`, you MUST have read:

| RFC | Why it matters |
|---|---|
| [RFC-0006](docs/rfcs/0006-binary-fidelity.md) | Binary fidelity — the non-negotiable constraint |
| [RFC-0019](docs/rfcs/0019-single-host-native-ipc.md) | UDS Hybrid IPC — the default transport architecture |
| [RFC-0021](docs/rfcs/0021-peripheral-design-and-synchronization.md) | Unified peripheral design — the "Gold Standard" |
| [RFC-0022](docs/rfcs/0022-fail-loudly-and-panic-linting.md) | Fail Loudly + linting reconciliation (`.expect`, `virtmcu-allow`) |
| [RFC-0023](docs/rfcs/0023-safe-qom-macros.md) | `#[qom_device]` — every peripheral uses it |
| [RFC-0024](docs/rfcs/0024-deterministic-routing-and-flow-control.md) | Assertion-based routing — unregistered packets panic |
| [RFC-0025](docs/rfcs/0025-zero-copy-transport.md) | `DataTransport::reserve()/commit()` zero-copy API |
| [RFC-0026](docs/rfcs/0026-zero-unsafe-qom-peripherals.md) | Zero unsafe in peripheral code |
| [RFC-0027](docs/rfcs/0027-cosim-bridge-raii-framework.md) | `CoSimBridge` — owns BQL yielding for bridges |
| [RFC-0031](docs/rfcs/0031-di-and-raii-mandate.md) | No globals, RAII, DI mandate |
| [RFC-0040](docs/rfcs/0040-testing-pyramid-and-emulation-verification.md) | Testing tiers and gate criteria |

Full RFC index: [docs/rfcs/README.md](docs/rfcs/README.md).

## Mandatory Pre-Flight Checklist
Before writing or modifying *any* code, you MUST output a brief plan that explicitly answers:
1. **Architectural Alignment:** Does this change rely on Dependency Injection (DI) and RAII?
2. **Fail Loudly:** If an invariant is violated in this new logic, does it `panic!`/`assert!` rather than warn?
3. **Verification Gate:** Which specific `make` or `virtmcu-test-runner` command will I run to empirically prove this works without breaking the deterministic simulation?

---

## Project specific guidelines

**virtmcu** is a **deterministic multi-node firmware simulation framework** built on QEMU:
1. **Dynamic QOM device plugins** (.so shared libraries).
2. **arm-generic-fdt machine** — ARM machines defined by Device Tree.
3. **Native VirtMCU QOM plugin** (`hw/rust/`) — deterministic clock and I/O.

## Single Source of Truth (SSOT) Mandate
- **Micro-Architecture**: All MMIO addresses, register offsets, and bitfields MUST be derived from a CMSIS-SVD file. Hardcoding these in Rust or C is BANNED.
- **Macro-Architecture**: Topology (which device is at which address) MUST be defined in the world YAML.
- **Unidirectional Generation**: Always use `virtmcu-cli` or `build.rs` to generate headers/constants. Never manually align "shadow" definitions.

## IMPORTANT REQUIREMENTS

**The same firmware ELF that runs on a real MCU must run unmodified in VirtMCU.**
- No virtmcu-specific startup code, linker sections, or compile-time flags in firmware.
- Peripherals mapped at the **exact** base addresses the real MCU datasheet specifies.
- Register layouts, reset values, and interrupt numbers must match physical silicon.

### Global Simulation Determinism

**Same topology YAML + same firmware ELFs + same `global_seed` → bit-identical output on every run.**

- **Topology declared, not discovered**: full network graph in world YAML, loaded by `DeterministicCoordinator` at startup. Runtime Zenoh peer-mode scouting is BANNED.
- **Canonical tie-breaking**: same-vtime messages delivered in order `(delivery_vtime_ns, source_node_id, sequence_number)` by the coordinator — never by OS scheduling.
- **Per-quantum barrier**: coordinator withholds quantum-Q messages until ALL nodes signal "quantum Q complete" (PDES barrier pattern).
- **Automated Synchronization (SOTA)**: The framework implicitly injects the `-S` flag to launch QEMU frozen, handles routing synchronization internally (`ensure_session_routing`), and issues `cont` via QMP. Do not manually call `ensure_session_routing` in tests.
- **Stochastic seeding**: derive per-node PRNG as `seed_for_quantum(global_seed, node_id, quantum_number)`. `rand::thread_rng()` and wall-clock seeding are BANNED.
- **Mobile nodes**: topology changes pushed by physics engine before each quantum step, never discovered at runtime.
- Any feature producing different output across identical runs is a VirtMCU bug.


## Clock Synchronization Model

| Mode | How to invoke | When to use |
|---|---|---|
| `standalone` | No `-device virtmcu-clock` | Rapid development, logic testing. |
| `slaved-suspend` | `-device virtmcu-clock,mode=slaved-suspend` | **Default.** Deterministic co-simulation. |
| `slaved-icount` | Same + `-icount shift=0,align=off,sleep=off` | Sub-quantum timing precision (PWM, µs). |

## Key Constraints

- **MMIO Delivery**: `mmio-socket-bridge` delivers **relative offsets**. Do NOT add the base address.
- **DTB Validation**: `yaml2qemu` validates all YAML peripherals are in the DTB — missing entries fail the build.
- **SysBus Mapping**: `-device`-only devices are NOT mapped into guest memory → Data Abort. Declare in YAML.
- **Topology-First**: full graph in `topology:` YAML before start. Coordinator rejects unlisted connections (logged as violations). Topology changes pushed by physics engine, not discovered.
- **Clock/Comms Separation**: clock sync (`ClockSyncTransport`) and emulated network (`DeterministicCoordinator`) use separate transports. Never mix.


## Language Selection Policy

- We use Rust as the primarily programming language.

---

## Production Engineering Mandates

### 1. Enterprise-Ready Quality (No Regression)
- Agents MUST NOT lower lint strictness, coverage, or security gates without explicit written human consent.
- In `--yolo` mode: only *increase* quality. Never suppress warnings (`#[allow(...)]`, `noqa`) or bypass the type system.

### 2. Peripherals
- **Simulation vs Boundary (CoSimBridge vs DeterministicReceiver)**:
  - If a peripheral participates in the simulation graph and respects virtual time (e.g., sensors, actuators, radios, SPI, wired buses), it MUST route all traffic through the `DeterministicCoordinator` using `DeterministicReceiver` (for ingress) and `reserve()/commit()` (for egress). Direct sockets bypass the PDES quantum barrier and are BANNED for simulation traffic.
  - If a peripheral is an infrastructure boundary that talks to external, non-simulation processes (e.g., test runners via UART/chardev, hardware-in-the-loop, or the coordinator itself), it MUST use `CoSimBridge`. `CoSimBridge` provides blocking I/O and BQL yielding for external processes that have no concept of virtual time.
- **Gold Standard**: All new peripherals MUST follow the `hw/rust/common/reference-peripheral` template exactly. This is the single source of truth for architectural patterns.
- **Ingress**: Use `DeterministicReceiver` for all incoming simulation data. `SafeSubscription` is STRICTLY BANNED in peripheral code (only allowed internally within framework primitives like `DeterministicReceiver` and backbone devices like `virtmcu-clock`).
- **MMIO Safety**: Use `VcpuDrain` and `MmioResult::wait_for` for blocking MMIO. Manual `condvar.wait()` loops are discouraged.
- **BQL (Big QEMU Lock)**: Agents MUST NOT manually acquire, release, or check the BQL. The framework (`virtmcu-qom`) handles BQL synchronization implicitly via `DeterministicReceiver` and `MmioDevice` traits. Direct calls to `bql_lock`, `bql_unlock`, or `virtmcu_is_bql_locked` are BANNED.

### 3. Safe Peripheral Teardown (Drain Pattern)

VirtMCU uses a **Drain Pattern** to ensure that no VCPUs are executing MMIO handlers when a peripheral is being destroyed (DSO unloading).

Mandatory integration:
1. **`VcpuDrain`**: Every peripheral state MUST include a `drain: VcpuDrain`.
2. **MMIO Guards**: Every `read`/`write` implementation MUST call `let _guard = self.drain.acquire();` at the very beginning.
3. **Blocking Reads**: Use `MmioResult::wait_for(...)`. This internally checks the drain state and ensures the VCPU thread can be unblocked for teardown.
4. **RAII Cleanup**: `DeterministicReceiver` and other resources MUST be stored in the state. Their `Drop` implementations handle unsubscription and cleanup automatically.

- **BANNED**: Manual `running` flags, manual thread joining in `Drop`, and bounded spinloops.
- Every new peripheral needs a shutdown integration test (teardown during blocked MMIO, no sanitizer errors).

### 4. Lessons Learned (Anti-Patterns — Do Not Repeat)

- **DSO Boundary Isolation Trap**: Never use Rust `static` or `static mut` (like `lazy_static!`, `Mutex`, `Atomic`) for state if peripherals might be compiled into separate `.so` files. Shared state must live in the QEMU main binary and be exported via `VIRTMCU_EXPORT`.
- **Single-Slot Global Callbacks**: Avoid single-slot function pointers (e.g., `void (*hook)(...)`). Always use chained arrays (e.g., `hook[8]`) or DI.
- **PDES Tie-Breaking**: Direct pub/sub between nodes is BANNED. All inter-node traffic routes through `DeterministicCoordinator`. Use `DeterministicReceiver` for all ingress.
- **BQL Usage**: Manual BQL management is BANNED. If you think you need it, you are violating the `MmioDevice`/`DeterministicReceiver` architecture.
- **Atomic State Transitions**: Use a single `AtomicU8` enum + `compare_exchange`. Multiple boolean flags allow illegal states.
- **Zenoh Executor Deadlocks**: Never block a Zenoh async thread. Offload to a background thread via `crossbeam_channel`.
- **UART FIFO Backpressure**: PL011 FIFO is 32 bytes. Check `qemu_chr_be_can_write`, buffer overflow in backlog, drain via `chr_accept_input`.
- **QEMU Patch Automation**: Never hand-edit `third_party/qemu`. All changes via `cargo run -p virtmcu-cli -- setup patch-qemu` or `apply_zenoh_hook.py`. This is enforced by a CI lint that rejects any modifications to `third_party` submodules unless corresponding changes exist in the `patches/` directory.
- **QOM Property Hyphens**: QEMU property names use **hyphens**, not underscores (e.g. `federation-id`, `coordinated-router`, `stall-timeout`). Underscores are silently ignored — the field stays null/default and the failure appears 30+ seconds later as `CLOCK_ERROR_STALL` or a coordinator federation_id mismatch. The test runner QEMU stderr monitor logs "UNKNOWN QOM PROPERTY DETECTED" when this happens, but the root fix is: always use hyphens in `-device` arguments and in `define_prop_*` registration strings.
- **Tokio Task Panics Are Silent**: `assert!` inside `tokio::spawn` kills the task, not the process. The write half of the connection is never stored; downstream code silently stalls waiting for a message that never arrives. Use `std::process::abort()` for invariant violations inside spawned coordinator tasks.
- **Quantum Pre-Increment Deadlock**: The PDES barrier protocol is `capture quantum → step_clock(quantum) → increment`. Pre-incrementing before `step_clock()` sends `quantum = current + 1`; the barrier's lookahead path silently buffers it and returns `Ok(None)` — no error, no log, no `sim/clock/start`. All nodes deadlock waiting for a release that never comes. The barrier now warns when all nodes land in lookahead simultaneously.
- **Env Var Reads in Peripherals Are BANNED**: Peripherals MUST NOT read `VIRTMCU_SIM_ID`. Pass the federation_id via a `federation-id` QOM property and call `UdsDataTransport::new_with_fed_id()`. `UdsDataTransport::new()` (env-var wrapper) is reserved for the production QEMU process where the parent sets the env. Reading env vars from peripheral code breaks concurrent tests and violates the DI mandate.

### 5. The "Fail Loudly" Principle (Crash-Only Design)
- **No Silent Failures**: Never catch an exception or `Result` just to log a warning and continue. If an internal invariant is violated, crash immediately.
- **Developer Errors vs Linting**: The codebase denies `clippy::panic` and `clippy::unwrap_used` but mandates crashing on invalid states. You MUST navigate this conflict as follows:
  - Prefer `.expect("reason")` for `Option`/`Result` since `clippy::expect_used` is explicitly allowed. NEVER use `.unwrap()`.
  - For `assert!`, `unreachable!`, or explicit `panic!` (e.g., unexpected enum variants, layout mismatches), you MUST add `#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"` to the top of the file.
- **User/Config Errors**: Return `Result::Err` or raise specific Exceptions for bad inputs, allowing the CLI boundary to print actionable help before exiting with `exit(1)`.
- **Warnings are Code Smell**: If a state is invalid enough to warrant a warning, it is invalid enough to be an error.

### 6. Architectural Patterns (RAII & Dependency Injection)
- **RAII (Resource Acquisition Is Initialization)**: All resources (memory, locks, file handles, Zenoh sessions) MUST be managed via RAII. Explicit `init()` and `deinit()`/`cleanup()` calls are BANNED for resource management; use constructors/destructors (Rust `Drop`, C++ destructors, Python context managers).
- **Dependency Injection (DI)**: Components MUST NOT hardcode or globally discover their dependencies (e.g., transports, coordinators, configs). Pass dependencies via constructors or factories. This is critical for deterministic testing and parallel safety.

---

## Before Every Commit — Mandatory Lint Gate

```bash
make test-check    # Fast-path: test-lint + test-unit (runs natively)
```

`[workspace.lints.clippy] all = "deny"` — every clippy warning is a build failure. `#[allow(clippy::...)]`, `#[allow(static_mut_refs)]`, and `#[allow(clippy::too_many_lines)]` are all BANNED in production code.

**Git hooks** (`pre-commit` + `pre-push`): run `make test-lint` (pre-commit) and `make test-unit` (pre-push) directly in the devcontainer shell. Install: `make install-git-hooks`. **Agents are PROHIBITED from skipping git hooks (`--no-verify` is disabled) during commit and push, unless explicitly permitted by a human.**

**Full CI parity before PR:** `make ci-check`. Complete pre-merge validation: `make ci-full`.

---

## Peripheral Porting Checklist

Use this checklist every time you port or create a peripheral in `hw/rust/`.

### QOM Properties
- [ ] All `define_prop_*` registration strings use **hyphens** (`federation-id`, `stall-timeout`), never underscores. QEMU silently ignores unknown property names.
- [ ] Every property that is required in a given mode is validated in `realize()` with `error_setg!` + `return` — never a later null-deref.
- [ ] Log the actual values used at startup via `virtmcu_qom::sim_info!("peripheral: node={} fed='{}'", ...)`. Log what you *used*, not what you configured.

### UDS Transport / federation_id
- [ ] Do **not** read `VIRTMCU_SIM_ID` in peripheral code. Use `UdsDataTransport::new_with_fed_id(path, node_id, fed_id_str)` where `fed_id_str` comes from the `federation-id` QOM property.
- [ ] Tests always use `new_with_fed_id()` with an explicit string — never `new()` + `std::env::set_var` in concurrent test contexts.

### Coordinator / PDES
- [ ] Invariant violations in `tokio::spawn` handlers use `std::process::abort()`, not `assert!` or `panic!`. A panicking detached task is a silent failure.
- [ ] `step_clock()` is called with `current_quantum` **before** incrementing. The counter increments **after** `try_join_all()` returns.
  ```rust
  let q = self.current_quantum;          // capture first
  try_join_all(nodes.map(|n| cc.step_clock(n, advance, vtime, q))).await?;
  self.current_quantum += 1;             // then increment
  ```

### Simulation vs Boundary
- [ ] Traffic that participates in virtual time (sensors, actuators, buses) routes through `DeterministicCoordinator` via `DeterministicReceiver` / `reserve()+commit()`.
- [ ] Infrastructure boundaries (UART test runner, HIL) use `CoSimBridge`.

### Teardown
- [ ] `drain: VcpuDrain` in state struct.
- [ ] `let _guard = self.drain.acquire();` at the top of every `read`/`write` handler.
- [ ] Blocking MMIO uses `MmioResult::wait_for(...)`.
- [ ] No manual `running` flags, no manual thread joins in `Drop`.

### Debugging stalls (`CLOCK_ERROR_STALL`)
When you see a stall, check in order:
1. **QEMU stderr**: look for "UNKNOWN QOM PROPERTY DETECTED" — underscore in property name.
2. **Coordinator stdout**: look for "FATAL: node federation_id" — federation-id mismatch (check property name hyphenation and that `new_with_fed_id` is used).
3. **Barrier warning**: look for "ALL N nodes submitted quantum=X as LOOKAHEAD" — quantum pre-increment bug in test runner.
4. **Coordinator task death**: look for absence of "node N registered" log — the connection handler task panicked silently.
5. **Timeout race**: `sim/clock/start` never arrives because the coordinator never received all `sim/coord/done` signals — check topology YAML matches the node count passed to `--nodes`.
