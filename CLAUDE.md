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
- \`1\` (STALL): QEMU failed to reach TB boundary within 60 s (likely firmware crash or deadlock).
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

