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
- `1` (STALL): QEMU failed to reach TB boundary within the stall timeout (default **5 s**; set `stall-timeout=<ms>` on the device — CI uses 60 000 ms via `VIRTMCU_STALL_TIMEOUT_MS`).
  - **CRITICAL NOTE:** QEMU **does not exit** on a stall. It logs "STALL DETECTED", replies with error code 1, and stays alive. The Python orchestrator is fully responsible for terminating the QEMU process if recovery is not desired.
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
- **Targeted Ninja Builds**: For the fastest turnaround when working on a specific peripheral, run ninja directly on the module target:
  `ninja -C third_party/qemu/build-virtmcu hw-virtmcu-<name>.so`
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

- **Centralized Versions**: Agents MUST adhere to the versions defined in the `VERSIONS` file for QEMU, Zenoh, and other core dependencies.
- **Verification**: Before suggesting or implementing upgrades, verify the current pinned versions in `VERSIONS` and `requirements.txt`.
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

**Mandates:**
- **Parallel Test Execution:** Our test suites run in parallel (using `pytest -n auto`). All tests MUST be designed to be isolated and idempotent. Avoid hardcoded ports, fixed UNIX socket paths, or shared global state that could cause interference between concurrent test runs.
- Never write complex setup/teardown loops in Bash. If a test requires booting QEMU alongside a background process (e.g., `zenoh_coordinator`), it **must** be written in Python using `pytest` fixtures to ensure safe teardown and prevent zombie processes.
- Internal logic (like CSMA/CA backoffs, flatbuffer serialization, struct alignment) **must** be tested natively in Rust using `#[test]`, bypassing QEMU boot entirely where possible.

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
- **DO NOT** automatically edit `.env` or `VERSIONS` files. These files contain specific version pins and local secrets (via symlinking) that should only be modified by the user or dedicated synchronization scripts (e.g., `make sync-versions`).
- **QEMU Patching:** All modifications to QEMU source files, C-level hooks, or SysBus definitions MUST be centralized in `scripts/apply-qemu-patches.sh`. Do not duplicate `sed` commands or `git am` calls across Dockerfiles or bare-metal setup scripts.

### 7. Environment Parity (1:1 Local-to-Remote Sync)
- **Mandate:** Local CI simulations (`make ci-local`) MUST exactly mirror the GitHub Actions environment. Tests and lints must be executed *inside* the isolated Docker container (`ghcr.io/refractsystems/virtmcu/devenv-base:dev-amd64`), never directly on the host machine.
- **Why:** This strict 1:1 parity prevents "Works on My Machine" bugs caused by host toolchain drift (e.g., a missing `cargo-audit` or `hadolint` version mismatch).
- **Reference:** For details on the CI architecture, caching strategies, and the `devenv-base` vs `devenv` split, consult `docs/CI_GUIDE.md`. Whenever modifying CI pipelines or `Makefile` targets, you must maintain this strict 1:1 synchronization constraint.

### 8. Enterprise-Ready Quality (No Quality Regression)
- **Mandate:** Agents are NEVER allowed to lower the quality, strictness, or coverage of lints, static analyzers, type-checkers, or security gates without explicit human written consent.
- **YOLO Mode Constraint:** Even when running in `--yolo` mode, agents can only *increase* software quality and enterprise-readiness on their own (e.g., by enabling stricter rules or fixing technical debt). They must never suppress warnings, disable lints (`#[allow(...)]`, `noqa`, etc.), or bypass the type system to resolve errors unless specifically instructed to do so for a verified edge case.

### 9. New Peripherals and the Educational C Model
- **Rust First**: All new peripheral models MUST be written in Rust using the `hw/rust/rust-dummy` template, unless they are specifically being ported from SystemC/C++.
- **Educational C Model**: We maintain one legacy C peripheral model (`hw/misc/educational-dummy.c`, instantiated as `dummy-device`) strictly for educational purposes and backward compatibility testing. It is automatically tested in Phase 2 integration tests to ensure we don't lose the ability to load traditional C plugins.


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
make lint     # Runs ruff, version checks, and cargo clippy (fails loudly)
```

### Git Hooks (Automation)
The project uses Git hooks to automatically run \`make lint\` on both \`commit\` and \`push\`.

- **Installation**:
  - **Devcontainer**: Hooks are installed automatically on container creation.
  - **Manual**: Run \`make install-hooks\` to set them up in your local \`.git/hooks\` directory.
- **Bypassing**: If you must commit/push without running lints (e.g., for a work-in-progress commit that you know is messy), use the \`--no-verify\` flag:
  \`\`\`bash
  git commit -m "wip: messy change" --no-verify
  git push --no-verify
  \`\`\`

**Pre-commit Hook**: A git \`pre-commit\` hook is installed that runs \`make lint\` automatically. If it fails, the commit will be blocked. Fix all lint/clippy errors before attempting to commit again.

---

## CI/CD Troubleshooting & "Make CI Green" Workflow (Enterprise CI Fixer Loop)

When instructed to **"fix CI"**, **"make CI green"**, or address pipeline failures, you MUST enter the **Enterprise CI Fixer Loop**. This is not just about pushing a fix; it is about absolute empirical certainty.

1. **Diagnose Remotely:** Use the GitHub CLI (`gh run list`, `gh run view --log`) to identify the exact failure.
2. **Local Reproduction (The `ci-full` Gate):** You MUST reproduce the failure locally using `make ci-full` or the corresponding containerized smoke test. If it passes locally but fails in CI, **STOP.** You have not reproduced it. Align local scripts, timeouts (`VIRTMCU_STALL_TIMEOUT_MS`), or environment until the failure is caught locally.
3. **Stress Test the Bug:** Once reproduced, "stress test the hell out of the bug." Run the failing test 100+ times (or use a dedicated stress test if one exists, or create a new one). Quantify the failure rate.
4. **Exhaustive Implementation:** Implement the fix.
5. **Verified Recovery:** Prove the fix works by running the same stress test again. It must reach a 100% success rate over a significant number of runs.
6. **Final Lint Gate:** Run `make ci-local` to ensure that lints, unit tests, and the lightweight environment are perfectly clean.
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
