# CLAUDE.md — qenode Project Context

This file is read automatically by Claude Code at session start.
Update it when architectural decisions change or new constraints are discovered.

---

## What This Project Is

**qenode** is an out-of-tree framework that makes QEMU behave like Renode.
Specifically, it provides:

1. **Dynamic QOM device plugins** — C/Rust peripheral models compiled as `.so` shared
   libraries, loadable into QEMU at runtime without recompiling the emulator.
2. **arm-generic-fdt machine** — ARM machines defined entirely by a Device Tree at runtime,
   eliminating hardcoded C machine structs. This requires the 33-patch patchew series from
   Ruslan Ruslichenko (submitted 2026-04-02) applied on top of QEMU 11.0.0-rc2.
3. **repl2qemu** — Python tool that parses Renode `.repl` platform description files and
   emits a `.dtb` (Device Tree Blob) + QEMU CLI command string.
4. **Robot Framework QMP library** — `qemu-keywords.robot` that maps Renode test keywords
   to QEMU Machine Protocol (QMP) JSON commands for CI/CD testing parity.
5. **vhost-user Python daemons** — Low-speed peripherals (I2C sensors, SPI config regs)
   implemented as standalone Python processes communicating over Unix sockets.

---

## QEMU Version and Patch Strategy

- **Base**: QEMU 11.0.0-rc2 (tag `v10.2.92` in `~/src/qemu`, git HEAD from upstream
  `https://gitlab.com/qemu-project/qemu.git`)
- **Required patches**: The 33-patch `arm-generic-fdt` series, patchew ID
  `20260402215629.745866-1-ruslichenko.r@gmail.com`
- **Fetch command** (when ready to apply):
  ```bash
  git fetch origin refs/for/master  # or fetch from patchew directly
  git am $(fetch-patchew-series 20260402215629.745866-1-ruslichenko.r@gmail.com)
  ```
- **Build flags** (must include):
  ```
  --enable-modules --enable-fdt --enable-plugins
  --target-list=arm-softmmu,arm-linux-user
  ```

**Do NOT** target RISC-V until ARM is fully validated. RISC-V expansion is Phase 2+.

---

## Dynamic Module Loading — Architecture Detail

Our devices are compiled as part of QEMU's Meson build (not truly out-of-tree at the
binary level, but source-managed in the qenode repo).

`scripts/setup-qemu.sh` creates a symlink:
```
~/src/qemu/hw/qenode  →  <qenode-repo>/hw
```
and appends `subdir('qenode')` to `~/src/qemu/hw/meson.build`.

Our `hw/meson.build` adds entries to QEMU's `modules` dict:
```meson
hw_qenode_modules += {'dummy': dummy_ss}
modules += {'hw-qenode': hw_qenode_modules}
```

With `--enable-modules`, this compiles to `hw-qenode-dummy.so` (Linux) or
`hw-qenode-dummy.dylib` (macOS), installed in `QEMU_MODDIR`.
QEMU's `module_info` table is auto-generated from compiled objects and includes our
device, so `-device dummy-device` auto-loads the `.so` without any `LD_PRELOAD` hack.

`scripts/run.sh` sets `QEMU_MODULE_DIR` to the installed module path and execs the
patched QEMU binary. No `LD_PRELOAD` needed.

---

## Directory Structure

```
qenode/
├── CLAUDE.md                  # This file — AI agent context
├── PLAN.md                    # Phased implementation plan with task checklist
├── README.md                  # Human-readable project overview
├── Makefile                   # Top-level: delegates to scripts/build.sh
├── docs/
│   ├── ARCHITECTURE.md        # Deep-dive: QEMU vs Renode analysis + target design
│   └── MIGRATION_GUIDE.md     # Step-by-step migration walkthrough per phase
├── hw/
│   └── dummy/
│       └── dummy.c            # Minimal QOM SysBusDevice — proves .so loading works
├── tools/
│   ├── repl2qemu/             # Python package: .repl → .dtb + QEMU CLI
│   │   ├── __init__.py
│   │   ├── parser.py          # Tokenizer + AST for .repl indent mode
│   │   ├── fdt_emitter.py     # AST → DTS text → invoke dtc → .dtb
│   │   └── cli_generator.py   # AST → QEMU CLI argument string
│   └── testing/
│       ├── qemu_keywords.robot  # Robot Framework resource: QMP-backed keywords
│       └── qmp_bridge.py        # Async QMP helper (wraps qemu.qmp library)
├── scripts/
│   ├── setup-qemu.sh          # Clone QEMU, apply patches, symlink hw/, build
│   └── run.sh                 # Launch wrapper: sets QEMU_MODULE_DIR
├── docker/
│   ├── Dockerfile             # Multi-stage build: patched QEMU + Python tools
│   └── docker-compose.yml     # Standalone test environment
└── requirements.txt           # Python: qemu.qmp, robotframework, lark, eclipse-zenoh
```

---

## Key Constraints

- **Development platform**: macOS and Linux. Windows is not supported. `setup-qemu.sh` actively drops `--enable-plugins` on macOS natively to avoid GLib module loading issues (GitLab #516). Use Docker on macOS when `--enable-plugins` is required.
- **C standard**: C11, matching QEMU's own style. Use QEMU's `qemu/osdep.h` as first include.
- **No `#define BUILD_DSO`**: This is not a QEMU macro. Don't use it.
- **QOM init pattern**: Use `OBJECT_DECLARE_SIMPLE_TYPE` + `DEFINE_TYPES()` macro (QEMU 7+).
  Do NOT use the old `type_register_static()` + `type_init()` pattern for new code.
- **QMP `query-cpus` is deprecated**: Use `query-cpus-fast` (deprecated since QEMU 4.x).
- **arm-generic-fdt is NOT in mainline QEMU**: It is in the patchew series only. Do not
  document it as if it is upstream until the patches are merged.
- **vhost-user is VirtIO-specific**: It cannot back arbitrary MMIO peripherals (UART, SPI,
  I2C) without a VirtIO transport in guest firmware. Use it only for GPIO/network devices
  or peripherals where a VirtIO transport already exists in the guest.
- **co-simulation (Verilator/EtherBone/Remote Port)**: Deferred to Phase 4. Do not implement
  or reference in Phases 1-3 code.

---

## Python Environment

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

Core dependencies:
- `qemu.qmp` — async QMP client
- `robotframework` — test harness
- `lark` — EBNF parser for .repl grammar

---

## Local Resources

- QEMU source: `~/src/qemu` (v10.2.92 / 11.0.0-rc2 pre-release, main branch)
- Renode source: `~/src/renode` (reference for .repl format and existing peripherals)
- QEMU headers needed for hw/: `~/src/qemu/include/`

---

## FirmwareStudio Integration and External Time Master

**This is the strategic north star for the project.**

qenode is the QEMU-layer component of **FirmwareStudio** (`~/src/FirmwareStudio`), a
digital twin platform for embedded firmware development. Understanding the full picture
is essential for making correct architectural decisions.

### The Big Picture

```
MuJoCo (physics)  ←→  TimeAuthority (Python)  ←→  NodeAgent (Python)
                                                         ↕  Unix socket
                                            QEMU (icount slave mode)
                                                         ↕  MMIO / QOM
                                               Firmware (bare-metal C)
                                                         ↕  IVSHMEM / Zenoh
                                           Physics sensors & actuators
```

### Key Architectural Decision: MuJoCo is the Time Master

**QEMU must run as a time slave.** Virtual time does NOT advance on its own.
Instead, the external `TimeAuthority` (running inside the MuJoCo container) sends a
`clock/advance/{delta_ns}` message via **Zenoh** once per `mj_step()`. QEMU advances
exactly `delta_ns` nanoseconds of virtual time and then blocks, waiting for the next
quantum. This guarantees that physics and firmware are always causally consistent —
firmware never runs ahead of or behind the physics simulation.

This is implemented via the **libqemu clock-socket patch** (`patches/0001-libqemu-clock-socket.patch`):
- QEMU opens a Unix socket (`/tmp/qemu-clock.sock`)
- A local `NodeAgent` Python daemon connects to that socket over Zenoh and forwards
  `ClockAdvance` payloads from the TimeAuthority
- The patch manipulates `qemu_icount_bias` to advance virtual time precisely

**Implication for all QEMU config**: `slaved-suspend` mode works best natively without icount. If using exact timer modes, `-icount shift=0,align=off,sleep=off` must be used.

### FirmwareStudio is a POC — Design is Flexible

FirmwareStudio's current design is a proof of concept. qenode can and should drive
design changes there. Flag anything that should change when writing Phase 7 code.

### Recommended FirmwareStudio Design Changes (when Phase 7 arrives)

| Current POC design | Recommended change | Reason |
|---|---|---|
| `apply_patch.py` code-injection approach | `patches/apply_libqemu.py` in qenode (done) | Reproducible, version-controlled, reviewable |
| `-icount` + `qemu_icount_bias` as the only clock mode | Add `slaved-suspend` (QMP stop/cont) as default | ~5x better performance for typical control loops |
| IVSHMEM PCI device for all sensor/actuator I/O | QOM peripheral models via arm-generic-fdt | Sensors defined in `.repl`, no hardcoded PCI setup |
| Hardcoded Cortex-A15 machine | `arm-generic-fdt` + `repl2qemu` | Any board from a `.repl` file |
| `node_agent.py` embedded in `cyber/src/` | `tools/node_agent/` in qenode | Single implementation, used by all worlds |
| `studio_server.py` coupling MCP to QEMU | Keep MCP as the AI/IDE layer; qenode exposes QMP | Separation of concerns |
| QEMU 10.2.1 pinned download | qenode-patched 11.0.0-rc2 image from `docker/Dockerfile` | Arm-generic-fdt patches, better APIs |

### What FirmwareStudio Currently Uses (to be replaced/improved by qenode)

| Current (FirmwareStudio) | Target (qenode) |
|---|---|
| Hardcoded Cortex-A15 `-M none` machine | `arm-generic-fdt` machine from .repl |
| IVSHMEM PCI device for sensor/actuator I/O | Proper QOM peripheral models |
| Placeholder `libqemu.c` patch (fake hashes) | Real, tested, upstreamable patch |
| Single machine type per Docker image | Any machine from a `.repl` file |
| Manual QEMU CLI construction | `repl2qemu` generates the CLI automatically |

### Zenoh as the Federation Bus

The message bus is **Eclipse Zenoh** (`eclipse-zenoh` Python package).
Key topics (from FirmwareStudio):
- `sim/clock/advance/{node_id}` — TimeAuthority → NodeAgent: advance by N ns
- `sim/eth/frame/ta/{node_id}` — TimeAuthority → NodeAgent: inject Ethernet frame
- `firmware/state` — NodeAgent → UI/API: sensor/actuator data

qenode's testing and tooling must support Zenoh. `requirements.txt` includes
`eclipse-zenoh`.

### Lessons from qbox (Qualcomm) and MINRES

Two prior art projects address the same problem of coupling QEMU to an external scheduler:

**Qualcomm qbox** (github.com/quic/qbox): Uses `libgssync` — a suspend/resume
synchronization policy library. QEMU is suspended at quantum boundaries via cooperative
hooks, does NOT use icount mode, and runs at full TCG speed between steps. This is the
model we should follow for the slaved-suspend clock mode.

**MINRES libqemu-cxx**: Wraps QEMU as a SystemC TLM-2.0 module. More invasive (requires
libqemu, which is not in upstream QEMU), but demonstrates the full co-simulation use case.
The key takeaway: tight SystemC integration is more than we need — our Zenoh-based
message bus already provides the equivalent of TLM-2.0 transactions over a network.

**What we adopt from qbox**: The suspend/resume approach for `slaved-suspend` mode.
The implementation: NodeAgent sends QMP `{"execute": "stop"}` before updating sensors,
updates IVSHMEM/MMIO, then sends `{"execute": "cont"}` to resume. No icount, full speed.

**What we skip**: Full SystemC/TLM-2.0 embedding. Our Zenoh + Unix socket approach is
simpler, works across containers/machines, and is sufficient for our use case.

### Phase 7 (planned) — FirmwareStudio Integration

Phase 7 will:
1. Formalize and test the `libqemu-clock-socket` patch (currently a placeholder in FirmwareStudio)
2. Add a `NodeAgent` class to `tools/` that bridges Zenoh ↔ QEMU Unix socket
3. Validate end-to-end: MuJoCo step → TimeAuthority → Zenoh → NodeAgent → QEMU → firmware response → sensors → physics

---

## Commit / PR Conventions

- Branch: `feature/<phase>-<short-description>`
- Commit format: `<scope>: <imperative description>` (e.g., `hw/dummy: add minimal QOM SysBusDevice`)
- One logical change per commit. Do not mix build system changes with C code changes.
