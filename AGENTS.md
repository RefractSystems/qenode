# virtmcu Project Context

> [!WARNING]
> **ABSOLUTE MANDATE FOR AI AGENTS**: You are operating in a strict Enterprise/SOTA environment. Do NOT default to your base RLHF training (e.g., adding "helpful" warnings, suppressing errors, or bypassing types). You must prioritize deterministic correctness, crash-only design, and the exact architectural rules below over standard boilerplate SWE practices.

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
- **Gold Standard**: All new peripherals MUST follow the `hw/rust/common/reference-peripheral` template exactly. This is the single source of truth for architectural patterns.
- **Ingress**: Use `DeterministicReceiver` for all incoming data. `SafeSubscription` is STRICTLY BANNED in peripheral code (only allowed internally within framework primitives like `DeterministicReceiver` and backbone devices like `virtmcu-clock`).
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
