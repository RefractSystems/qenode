# CLAUDE.md — virtmcu Project Context

This file is read automatically by Claude Code and Gemini CLI at session start
(`GEMINI.md` is a symlink to this file — maintain only this one).
Update it when architectural decisions change or new constraints are discovered.

---

## What This Project Is

**virtmcu** is a **deterministic multi-node firmware simulation framework** built on QEMU.
It is the QEMU layer of **FirmwareStudio**, a digital twin platform for embedded systems
where a physics engine (MuJoCo) drives the simulation clock and QEMU runs firmware in
lockstep with the physical world. Specifically, it provides:

1. **Dynamic QOM device plugins** — C/Rust peripheral models compiled as `.so` shared
   libraries, loadable into QEMU at runtime without recompiling the emulator.
2. **arm-generic-fdt machine** — ARM machines defined entirely by a Device Tree at runtime,
   eliminating hardcoded C machine structs. Requires the 33-patch patchew series from
   Ruslan Ruslichenko (submitted 2026-04-02) applied on top of QEMU 11.0.0-rc3.
3. **Native Zenoh QOM plugin** (`hw/zenoh/`) — C/Rust modules compiled into QEMU that link
   `zenoh-c` directly: cooperative time slaving, deterministic multi-node Ethernet and UART,
   and clock synchronization with the external TimeAuthority.
4. **repl2qemu / yaml2qemu** — Python tools that parse Renode `.repl` or OpenUSD-aligned
   `.yaml` board descriptions and emit a `.dtb` + QEMU CLI string.
5. **Robot Framework QMP library** — `qemu_keywords.robot` that maps Renode test keywords
   to QEMU Machine Protocol (QMP) JSON commands for CI/CD testing parity.

---

## Phase Status

| Phase | Description | Status |
|-------|-------------|--------|
| 0 | Repository setup, documentation | **Done** |
| 1 | QEMU build with arm-generic-fdt patches | **Done** |
| 2 | Dynamic QOM plugin infrastructure | **Done** |
| 3 | repl2qemu parser (.repl → .dtb + QEMU CLI) | **Done** |
| 3.5 | YAML Platform & OpenUSD Alignment | **Done** |
| 4 | Robot Framework QMP library | **Done** |
| 5 | Co-Simulation Bridge (SystemC / Remote Port) | **Done** |
| 6 | Multi-node Zenoh network coordinator | **Done** |
| 7 | Native Zenoh clock plugin + FirmwareStudio integration | **Done** |
| 8 | Deterministic multi-node UART (zenoh-chardev) | **Done** |
| 9 | Advanced co-simulation (shared physical media) | **Done** |
| 10 | Sensor/Actuator Abstraction Layers (SAL/AAL) | To Do |

See `PLAN.md` for the per-task checklist.

---

## QEMU Version and Patch Strategy

- **Base**: QEMU 11.0.0-rc3 (tag `v11.0.0-rc3` in `third_party/qemu`)
- **Patches applied in order by `scripts/setup-qemu.sh`** (and equivalently in the Dockerfile):
  1. `patches/arm-generic-fdt-v3.mbx` — 33-patch arm-generic-fdt series (patchew ID
     `20260402215629.745866-1-ruslichenko.r@gmail.com`) applied via `git am`
  2. `patches/apply_zenoh_hook.py` — AST-injects `virtmcu_tcg_quantum_hook` function
     pointer into `accel/tcg/cpu-exec.c`; also creates `include/virtmcu/hooks.h`
  3. `patches/apply_zenoh_netdev.py` — registers the Zenoh netdev backend in `net/net.c`
  4. `patches/apply_zenoh_chardev.py` — registers the Zenoh chardev backend in `chardev/char.c`
  5. `patches/apply_zenoh_qapi.py` — adds `NetdevZenohOptions` and `ChardevZenohOptions`
     to QEMU's QAPI schemas (`qapi/net.json`, `qapi/char.json`)

- **Dockerfile** runs patches 1, 2, 5 directly (clones QEMU fresh; patches 3 & 4 run
  via `setup-qemu.sh` for local builds — ensure parity when adding new patches).

- **Required configure flags**:
  ```
  --enable-modules --enable-fdt
  --target-list=arm-softmmu
  ```
  `--enable-plugins` is NOT included: it conflicts with `--enable-modules` on macOS
  (GLib issue, GitLab #516) and is not needed for the virtmcu module approach.

**Do NOT** target RISC-V until ARM is fully validated. RISC-V expansion is Phase 2+.

---

## Three Clock Modes

All clock behaviour is controlled by whether a `zenoh-clock` device is present and which
flags are passed:

| Mode | How to invoke | Throughput | When to use |
|---|---|---|---|
| `standalone` | No `-device zenoh-clock` | **100%** — full TCG speed | Development, CI without physics |
| `slaved-suspend` | `-device zenoh-clock,node=N,router=...` | **~95%** — pauses only at TB boundaries | **Default for FirmwareStudio.** Control loops ≥ one quantum. |
| `slaved-icount` | Same + `-icount shift=0,align=off,sleep=off` | **~15–20%** | Firmware measures sub-quantum intervals (PWM, µs DMA). |

The mode is implemented in `hw/zenoh/zenoh-clock.c` via the `virtmcu_tcg_quantum_hook`
function pointer injected into `cpu-exec.c`.

**BQL constraint**: The Zenoh `GET` call that blocks at each quantum boundary must be made
with the Big QEMU Lock released. Blocking while holding the BQL deadlocks the QEMU
process — the main event loop (QMP, GDB stub, chardev I/O) cannot acquire the lock.

```c
bql_unlock();
zenoh_reply = zenoh_get(queryable);  /* blocks here */
bql_lock();
/* now safe to update timers_state */
```

---

## Dynamic Module Loading — Architecture Detail

`scripts/setup-qemu.sh` creates a symlink:
```
third_party/qemu/hw/virtmcu  →  <virtmcu-repo>/hw
```
and appends `subdir('virtmcu')` to `third_party/qemu/hw/meson.build`.

`hw/meson.build` adds entries to QEMU's `modules` dict:
```meson
modules += {'hw-virtmcu': hw_virtmcu_modules}
```

With `--enable-modules`, this compiles to `hw-virtmcu-<name>.so` (Linux),
installed in `QEMU_MODDIR`. QEMU's `module_info` table includes our devices, so
`-device my-device` auto-loads the `.so` without any `LD_PRELOAD` hack.

---

## Directory Structure

```
virtmcu/
├── CLAUDE.md                   # This file — AI agent context (GEMINI.md → symlink)
├── GEMINI.md                   # Symlink to CLAUDE.md — do not edit separately
├── PLAN.md                     # Phased task checklist
├── README.md                   # Human-readable project overview
├── CONTRIBUTING.md             # Setup, dev workflow, code style
├── pyproject.toml              # ruff lint/format config (E501 ignored, line-length=120)
├── Makefile                    # Top-level: setup, build, run, lint, fmt, venv, test
│
├── hw/                         # C/Rust QOM peripheral models (no Python in sim loop)
│   ├── meson.build             # Integrates hw/ into QEMU's module build
│   ├── dummy/dummy.c           # Minimal QOM SysBusDevice — start here (C)
│   ├── rust-dummy/             # Minimal QOM SysBusDevice — start here (Rust FFI)
│   ├── misc/
│   │   └── mmio-socket-bridge.c # [Phase 5] SystemC/Remote Port co-simulation bridge
│   └── zenoh/                  # [Phase 7+] Native Zenoh QOM plugin
│       ├── zenoh-clock.c       # TCG cooperative halt + Zenoh clock sync (Phase 7)
│       ├── zenoh-netdev.c      # Deterministic multi-node Ethernet backend (Phase 7)
│       └── zenoh-chardev.c     # Deterministic multi-node UART backend (Phase 8)
│
├── tools/
│   ├── repl2qemu/              # .repl → Device Tree + QEMU CLI (offline, no QEMU)
│   │   ├── __init__.py
│   │   ├── parser.py           # Tokenizer + AST for .repl indent grammar
│   │   ├── fdt_emitter.py      # AST → DTS → invoke dtc → .dtb
│   │   └── cli_generator.py    # AST → QEMU CLI argument string
│   ├── repl2yaml.py            # Renode .repl → OpenUSD-aligned .yaml (offline)
│   ├── yaml2qemu.py            # .yaml board description → .dtb + QEMU CLI
│   ├── qmp_probe.py            # Interactive QOM tree explorer over QMP
│   ├── systemc_adapter/        # [Phase 5] C++ SystemC TLM-2.0 ↔ Remote Port bridge
│   └── testing/                # Python test harness (drives QEMU externally, not inline)
│       ├── qemu_keywords.robot # Robot Framework compatibility layer
│       ├── QemuLibrary.py      # Robot Framework library implementation
│       ├── qmp_bridge.py       # Async QMP helper (wraps qemu.qmp)
│       ├── test_qmp.py         # pytest primary QMP integration suite
│       └── conftest.py         # pytest fixtures and QEMU launcher
│
├── patches/
│   ├── arm-generic-fdt-v3.mbx  # 33-patch series (apply with git am)
│   ├── arm-generic-fdt-v3.cover # Cover letter for the patch series
│   ├── apply_zenoh_hook.py     # Injects virtmcu_tcg_quantum_hook into cpu-exec.c
│   ├── apply_zenoh_netdev.py   # Registers Zenoh netdev in net/net.c
│   ├── apply_zenoh_chardev.py  # Registers Zenoh chardev in chardev/char.c
│   └── apply_zenoh_qapi.py     # Adds NetdevZenohOptions + ChardevZenohOptions to QAPI
│
├── test/                       # Phase smoke tests (run inside Docker)
│   ├── phase1/ … phase8/       # smoke_test.sh per phase
│
├── tests/                      # Python unit tests (no QEMU required)
│
├── scripts/
│   ├── setup-qemu.sh           # Clone QEMU, apply patches, symlink hw/, build
│   └── run.sh                  # Launch wrapper: sets QEMU_MODULE_DIR
│
├── docker/
│   ├── Dockerfile              # Multi-stage: toolchain / builder / runtime
│   └── docker-compose.yml      # Standalone test environment
│
├── docs/
│   ├── ARCHITECTURE.md         # Full technical deep-dive (read this first)
│   └── MIGRATION_GUIDE.md      # Step-by-step migration per phase
│
└── requirements.txt            # Python: qemu.qmp, robotframework, lark, eclipse-zenoh
```

---

## Key Constraints

- **Development platform**: macOS and Linux. Windows is not supported.
- **C standard**: C11, matching QEMU's own style. First include must be `#include "qemu/osdep.h"`.
- **`BUILD_DSO` is a real QEMU macro**: QEMU's build system defines `-DBUILD_DSO` when
  compiling any shared module. Do NOT use `#ifdef BUILD_DSO` to gate virtmcu-specific code
  — it is set for all modules, upstream and ours alike. Use QOM type checks instead.
- **QOM init pattern**: Use `OBJECT_DECLARE_SIMPLE_TYPE` + `DEFINE_TYPES()` macro (QEMU 7+).
  Do NOT use the old `type_register_static()` + `type_init()` pattern for new code.
- **QMP `query-cpus` is deprecated**: Use `query-cpus-fast`.
- **arm-generic-fdt is NOT in mainline QEMU**: It is in the patchew series only. Do not
  document it as if it were upstream until the patches are merged.
- **vhost-user is VirtIO-specific**: It cannot back arbitrary MMIO peripherals (UART, SPI,
  I2C) without a VirtIO transport in the guest firmware. Use it only for GPIO/network
  devices where a VirtIO transport already exists in the guest.
- **No Python in the Simulation Loop**: Python is strictly banned from QEMU's execution
  runtime. No Python daemons, no vhost-user Python backends, no Python processes bridging
  sockets at quantum boundaries. All peripherals, time-sync, and networking must be native
  C/Rust QOM modules. Python is only permitted for offline tooling (repl2qemu, pytest).
- **QAPI schema changes**: Adding a new netdev/chardev backend requires adding the
  corresponding struct/enum/union entries to `patches/apply_zenoh_qapi.py`. Do NOT only
  modify `third_party/qemu/qapi/*.json` — the Docker build clones QEMU fresh and needs
  the patch script to apply schema changes.
- **co-simulation (Verilator/EtherBone/Remote Port)**: Phase 5 (done). Phase 9 expands
  this to shared physical media (e.g. CAN) via async IRQs and multi-threaded adapters.
- **Cyber-Physical Bridge (SAL/AAL)**: Telemetry and physics data must flow through
  Sensor/Actuator Abstraction Layers. Phase 10.
- **Standalone Telemetry (RESD)**: Use Renode Sensor Data format for deterministic data
  ingestion in standalone CI/CD modes (no physics engine required).
- **Integrated Mode Physics**: Use zero-copy shared memory (`mjData`) for MuJoCo or the
  Accellera Federated Simulation Standard (FSS) for OpenUSD/NVIDIA Omniverse.
- **Deterministic UART**: Serial communication across nodes uses `zenoh-chardev.c` with
  virtual timestamps. Phase 8.

---

## Python Environment

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

Core dependencies: `qemu.qmp`, `robotframework`, `lark`, `eclipse-zenoh`

Lint config lives in `pyproject.toml` (ruff, `E,F,W,I` rules, `E501` ignored,
`line-length=120`).

---

## Local Resources

- QEMU source: `third_party/qemu` (v11.0.0-rc3 with virtmcu patches applied)
- Renode source: `third_party/renode` (reference for .repl format and existing peripherals)
- QEMU headers needed for hw/: `third_party/qemu/include/`

---

## FirmwareStudio Integration and External Time Master

**This is the strategic north star for the project.**

virtmcu is the QEMU-layer component of **FirmwareStudio**, a digital twin platform for
embedded firmware development.

### The Big Picture

```
MuJoCo (physics)  ←→  TimeAuthority (Python)
                               ↕  Zenoh GET sim/clock/advance/{node_id}
                       QEMU + hw/zenoh/ plugin  (native C, no Python middleman)
                               ↕  TCG cooperative suspend/resume (BQL released during wait)
                          Firmware (bare-metal C)
                               ↕  QOM peripheral models (SAL/AAL boundary)
                       Physics sensors & actuators
```

Multi-node: each QEMU instance runs the same `hw/zenoh/` plugin. Ethernet frames and UART
bytes carry virtual timestamps and are delivered to the guest only when virtual time reaches
the stamped arrival time — deterministic by construction, not by coordination.

### Key Architectural Decision: MuJoCo is the Time Master

**QEMU must run as a time slave.** Virtual time does NOT advance on its own.
The external `TimeAuthority` sends a `GET sim/clock/advance/{node_id}` Zenoh message once
per `mj_step()`. QEMU advances exactly `delta_ns` nanoseconds and then blocks waiting for
the next quantum. Wire protocol:

```
GET sim/clock/advance/{node_id}
  payload → { uint64 delta_ns; uint64 mujoco_time_ns; }
  reply   ← { uint64 current_vtime_ns; uint32 n_pending_frames; }
```

### Zenoh Topics

| Topic | Direction | Description |
|---|---|---|
| `sim/clock/advance/{node_id}` | TimeAuthority → zenoh-clock.c | Advance virtual clock by N ns |
| `sim/eth/frame/{node_id}/tx` | zenoh-netdev.c → coordinator | Outbound Ethernet frame |
| `sim/eth/frame/{node_id}/rx` | coordinator → zenoh-netdev.c | Inbound frame + delivery vtime |
| `sim/serial/{node_id}/tx` | zenoh-chardev.c → peer | UART bytes + virtual timestamp |
| `sim/serial/{node_id}/rx` | peer → zenoh-chardev.c | Inbound UART bytes + delivery vtime |
| `firmware/state` | optional telemetry | UI/API (out of scope for Phases 1–6) |

### FirmwareStudio is a POC — Design is Flexible

FirmwareStudio's current design is a proof of concept. virtmcu can and should drive
design changes there. Flag anything that should change when writing Phase 7+ code.

### Recommended FirmwareStudio Design Changes (when Phase 7 arrives)

| Current POC design | Recommended change | Reason |
|---|---|---|
| `-icount` + `qemu_icount_bias` as the only clock mode | `slaved-suspend` as default | ~95% free-run speed; no external Python process |
| IVSHMEM PCI device for all sensor/actuator I/O | QOM peripheral models via arm-generic-fdt | Sensors defined in `.repl`, no hardcoded PCI setup |
| Hardcoded Cortex-A15 machine | `arm-generic-fdt` + `repl2qemu` | Any board from a `.repl` file |
| `node_agent.py` embedded in `cyber/src/` | **Deleted** — replaced by `hw/zenoh/` native plugin | Eliminates Python from the simulation loop entirely |
| `studio_server.py` coupling MCP to QEMU | Keep MCP as the AI/IDE layer; virtmcu exposes QMP | Separation of concerns |
| QEMU 10.2.1 pinned download | virtmcu-patched 11.0.0-rc3 image from `docker/Dockerfile` | arm-generic-fdt patches, better APIs |

### Lessons from qbox (Qualcomm) and MINRES

**Qualcomm qbox** (github.com/quic/qbox): Uses `libgssync` — cooperative suspend/resume at
quantum boundaries via hooks. Does NOT use icount. Runs at full TCG speed between steps.
This is the model for `slaved-suspend` mode.

**MINRES libqemu-cxx**: Wraps QEMU as a SystemC TLM-2.0 module. More invasive than we need.
Key takeaway: our Zenoh message bus already provides TLM-2.0-equivalent transactions over
a network, without the SystemC dependency.

**What we adopt from qbox**: Cooperative suspend/resume — hooking into the TCG loop at
quantum boundaries and blocking the QEMU thread until the external scheduler replies via
Zenoh (rather than SystemC). Same TB-boundary precision, no in-process SystemC.

---

## Commit / PR Conventions

- Branch: `feature/<phase>-<short-description>`
- Commit format: `<scope>: <imperative description>` (e.g., `hw/zenoh: add chardev backend`)
- One logical change per commit. Do not mix build system changes with C code changes.

## Before Every Push — Mandatory Lint Gate

**Always run `make fmt` followed by `make lint` before pushing or creating a PR.**

```bash
make fmt   # auto-fixes formatting and fixable style errors (ruff format + ruff check --fix)
make lint  # fails loudly if anything remains (same check as CI tier 1)
```

- `make fmt` is safe to run at any time — it only touches Python files under `tools/`, `tests/`, `patches/`.
- `make lint` mirrors the `ruff check` step in `.github/workflows/ci.yml` exactly (reads `pyproject.toml`).
- If lint fails after `make fmt`, fix the remaining issues manually before pushing.
- Never bypass this step — a lint failure blocks the entire CI build matrix.
