# CLAUDE.md — virtmcu Project Context

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
3. **Native Zenoh QOM plugin** (\`hw/zenoh/\`) — deterministic clock and I/O.
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
- \`0\` (OK): Quantum completed successfully.
- \`1\` (STALL): QEMU failed to reach TB boundary within the stall timeout (default **5 s**; set `stall-timeout=<ms>` on the device — CI uses 60 000 ms via `VIRTMCU_STALL_TIMEOUT_MS`).
- \`2\` (ZENOH_ERROR): Transport layer failure.

---

## Timing Model and Constraints

### 1. MMIO Socket Blocking
When using \`mmio-socket-bridge\`, every MMIO read/write blocks the QEMU TCG thread in a synchronous socket syscall.
- **CPU State**: The emulated CPU is **Halted** while waiting for the server.
- **icount Advancement**: Virtual time does NOT advance while blocked in a bridge call.
- **Latency Impact**: High bridge latency can cause clock stalls. Ensure the socket server is performant.

### 2. WFI (Wait For Interrupt) behavior
- In \`slaved-suspend\`, virtual time advances while the CPU is in WFI.
- The next quantum boundary will still trigger a clock-halt even if the CPU is idling.
- **Best Practice**: Use ARM Generic Timer interrupts at 100Hz rather than tight polling loops for control.

---

## Key Constraints

- **MMIO Delivery**: \`mmio-socket-bridge\` delivers **relative offsets** to the socket. External models should NOT include the base address in their match logic.
- **DTB Validation**: \`yaml2qemu\` validates that every peripheral defined in YAML is correctly mapped in the output DTB. If a mapping is missing, build will fail.
- **SysBus Mapping**: Devices added via \`-device\` only (not in YAML) are **NOT mapped** into guest memory. They will cause Data Aborts.

---

## Directory Structure

```
virtmcu/
├── hw/                         # C/Rust QOM peripheral models
│   ├── misc/
│   │   └── mmio-socket-bridge.c # Offset-based Unix socket bridge
│   └── zenoh/
│       └── zenoh-clock.c       # Clock sync with error reporting (Migrating to Rust)
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
| **Sim Loop** | **Rust** (Pref) / **C** | **NATIVE ONLY.** No Python bridges. |
| **Physics/SystemC** | **C++** | Standard for TLM-2.0 / MuJoCo. |
| **Tooling/Parsing** | **Python** | Out-of-band only. |
| **Telemetry** | **Rust** | Direct FlatBuffers/Zenoh integration. |

**Banned:** Python in the hot simulation loop (MMIO/Clock/Netdev).
**Recommended:** Migrate `hw/zenoh/*.c` to native Rust (Phase 18) to eliminate `zenoh-c` FFI.

---

## Production Engineering Mandates

To ensure the highest level of professional software engineering, all agents MUST adhere to these standards:

### 1. Environment Agnosticism (Zero Hardcoded Paths)
- **NO absolute paths** (e.g., `/Users/marcin/...`) or user-specific home directory references.
- Use **relative paths** based on the project root or the current file's location.
- Use platform-appropriate path joining (e.g., `os.path.join` in Python, `path::PathBuf` in Rust, `std::filesystem` in C++).
- Leverage environment variables for system-specific configuration.

### 2. Explicit Constants (No Magic Numbers)
- **BANNED:** Inline literal numbers without clear context (e.g., `delay(500)`, `buffer[1024]`).
- **Required:** Define named constants or enums with clear documentation explaining the value's origin (e.g., from a datasheet, a performance budget, or a protocol spec).
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


## CI/CD Troubleshooting & "Make CI Green" Workflow

When instructed to "fix CI", "make CI green", or address pipeline failures, you MUST follow this autonomous loop until success:

1. **Diagnose Remotely:** Use the GitHub CLI (`gh run list`, `gh run view --log`) to identify the exact failure. Always use `gh` to avoid GitHub API rate limits.
2. **Reproduce Locally:** BEFORE making code changes, run the corresponding test locally to reproduce the error.
3. **Align Local with Remote (Crucial):** If the step fails in CI but passes locally, DO NOT fix the code yet. First, modify the local test scripts, `Makefile`, or environment to ensure the failure reproduces locally. **Our local tests must catch what CI catches.**
4. **Fix & Verify:** Implement the fix and verify it passes the newly aligned local test suite.
5. **Push:** Commit and push the changes.
6. **Monitor & Loop:** Autonomously monitor the new CI run (e.g., using `gh run watch`). If it fails, immediately restart this loop. Do not stop or prompt the user until all checks are officially green.
