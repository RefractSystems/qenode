# CLAUDE.md — virtmcu Project Context

This file is read automatically by Claude Code at session start.
Update it when architectural decisions change or new constraints are discovered.

---

## What This Project Is

**virtmcu** is an out-of-tree framework that makes QEMU behave like Renode.
Specifically, it provides:

1. **Dynamic QOM device plugins** — C/Rust peripheral models compiled as `.so` shared
   libraries, loadable into QEMU at runtime without recompiling the emulator.
2. **arm-generic-fdt machine** — ARM machines defined entirely by a Device Tree at runtime,
   eliminating hardcoded C machine structs. This requires the 33-patch patchew series from
   Ruslan Ruslichenko (submitted 2026-04-02) applied on top of QEMU 11.0.0-rc3.
3. **repl2qemu** — Python tool that parses Renode `.repl` platform description files and
   emits a `.dtb` (Device Tree Blob) + QEMU CLI command string.
4. **Robot Framework QMP library** — `qemu-keywords.robot` that maps Renode test keywords
   to QEMU Machine Protocol (QMP) JSON commands for CI/CD testing parity.
5. **Native Zenoh QOM plugin** (`hw/zenoh/`) — C/Rust module compiled into QEMU that links
   `zenoh-c` directly, hooks into the TCG execution loop for cooperative suspend/resume,
   and acts as the multi-node network backend. Replaces the Python node_agent entirely.

---

## QEMU Version and Patch Strategy

- **Base**: QEMU 11.0.0-rc3 (tag `v11.0.0-rc3` in `third_party/qemu`, git HEAD from upstream
  `https://gitlab.com/qemu-project/qemu.git`)
- **Required patches** (applied in order by `scripts/setup-qemu.sh`):
  1. The 33-patch `arm-generic-fdt` series (patchew ID
     `20260402215629.745866-1-ruslichenko.r@gmail.com`) — applied via `git am`
  2. `patches/apply_libqemu.py` — AST-injects icount bias Unix socket into `cpus.c`
     and `icount.c` (stepping stone for slaved-icount; superseded by hw/zenoh/ in Phase 7)
  3. `patches/apply_zenoh_hook.py` — AST-injects a `virtmcu_tcg_quantum_hook` function
     pointer into `cpu-exec.c`, called at every TB boundary. Required because QEMU
     exports no extensibility API for QOM modules to hook the internal TCG loop.
- **Build flags** (must include):
  ```
  --enable-modules --enable-fdt --enable-plugins
  --target-list=arm-softmmu,arm-linux-user
  ```

**Do NOT** target RISC-V until ARM is fully validated. RISC-V expansion is Phase 2+.

---

## Dynamic Module Loading — Architecture Detail

Our devices are compiled as part of QEMU's Meson build (not truly out-of-tree at the
binary level, but source-managed in the virtmcu repo).

`scripts/setup-qemu.sh` creates a symlink:
```
third_party/qemu/hw/virtmcu  →  <virtmcu-repo>/hw
```
and appends `subdir('virtmcu')` to `third_party/qemu/hw/meson.build`.

Our `hw/meson.build` adds entries to QEMU's `modules` dict:
```meson
hw_virtmcu_modules += {'dummy': dummy_ss}
modules += {'hw-virtmcu': hw_virtmcu_modules}
```

With `--enable-modules`, this compiles to `hw-virtmcu-dummy.so` (Linux) or
`hw-virtmcu-dummy.dylib` (macOS), installed in `QEMU_MODDIR`.
QEMU's `module_info` table is auto-generated from compiled objects and includes our
device, so `-device dummy-device` auto-loads the `.so` without any `LD_PRELOAD` hack.

`scripts/run.sh` sets `QEMU_MODULE_DIR` to the installed module path and execs the
patched QEMU binary. No `LD_PRELOAD` needed.

---

## Directory Structure

```
virtmcu/
├── CLAUDE.md                  # This file — AI agent context
├── PLAN.md                    # Phased implementation plan with task checklist
├── README.md                  # Human-readable project overview
├── Makefile                   # Top-level: delegates to scripts/build.sh
├── docs/
│   ├── ARCHITECTURE.md        # Deep-dive: QEMU vs Renode analysis + target design + ADRs
│   └── MIGRATION_GUIDE.md     # Step-by-step migration walkthrough per phase
├── hw/
│   ├── dummy/
│   │   └── dummy.c            # Minimal QOM SysBusDevice — proves .so loading works
│   └── zenoh/                 # [Phase 7] Native Zenoh QOM plugin
│       ├── zenoh-clock.c      # TCG cooperative halt + Zenoh clock sync
│       └── zenoh-netdev.c     # Custom -netdev backend for deterministic multi-node
├── tools/
│   ├── repl2qemu/             # Python package: .repl → .dtb + QEMU CLI (offline only)
│   │   ├── __init__.py
│   │   ├── parser.py          # Tokenizer + AST for .repl indent mode
│   │   ├── fdt_emitter.py     # AST → DTS text → invoke dtc → .dtb
│   │   └── cli_generator.py   # AST → QEMU CLI argument string
│   ├── testing/               # Python test harness (drives QEMU externally, not inline)
│   │   ├── qemu_keywords.robot  # Robot Framework compatibility layer
│   │   └── qmp_bridge.py        # Async QMP helper (wraps qemu.qmp library)
│   └── node_agent/            # DEPRECATED — superseded by hw/zenoh/ in Phase 7
│                              # Kept as reference for wire protocol only
├── patches/
│   ├── arm-generic-fdt-v3.mbx  # 33-patch series (apply with git am)
│   ├── apply_libqemu.py        # Injects icount bias socket (Phase 1-6 stepping stone)
│   └── apply_zenoh_hook.py     # Injects virtmcu_tcg_quantum_hook function pointer
│                               # into cpu-exec.c; required for hw/zenoh/zenoh-clock.c
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
- **No Python in the Simulation Loop**: Python is strictly banned from QEMU's execution
  runtime. No Python daemons, no vhost-user Python backends, no node_agent.py bridging
  sockets at quantum boundaries. All peripherals, time-sync, and networking must be native
  C/Rust QOM modules. Python is only permitted for offline tooling (repl2qemu, pytest).
- **co-simulation (Verilator/EtherBone/Remote Port)**: Deferred to Phase 5. Phase 9 expands this to cover Shared Physical Media (e.g. CAN) via async IRQs and multi-threaded adapters.
- **Cyber-Physical Bridge (SAL/AAL)**: Telemetry and physics data must flow through Sensor/Actuator Abstraction Layers.
- **Standalone Telemetry (RESD)**: Use Renode Sensor Data format for deterministic data ingestion in standalone CI/CD modes.
- **Integrated Mode Physics**: Use zero-copy shared memory (`mjData`) for MuJoCo or the Accellera Federated Simulation Standard (FSS) for OpenUSD/NVIDIA Omniverse.
- **Deterministic UART**: Serial communication across nodes must use the native `zenoh-chardev.c` plugin with virtual timestamps (Phase 8).

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

- QEMU source: `third_party/qemu` (v11.0.0-rc3, tag `v11.0.0-rc3`)
- Renode source: `third_party/renode` (reference for .repl format and existing peripherals)
- QEMU headers needed for hw/: `third_party/qemu/include/`

---

## FirmwareStudio Integration and External Time Master

**This is the strategic north star for the project.**

virtmcu is the QEMU-layer component of **FirmwareStudio** (an upstream digital twin repo), a
digital twin platform for embedded firmware development. Understanding the full picture
is essential for making correct architectural decisions.

### The Big Picture

```
MuJoCo (physics)  ←→  TimeAuthority (Python)
                               ↕  Zenoh
                       QEMU + hw/zenoh/ plugin  (native C/Rust, no Python middleman)
                               ↕  TCG cooperative suspend/resume
                          Firmware (bare-metal C)
                               ↕  QOM peripheral models
                       Physics sensors & actuators
```

### Key Architectural Decision: MuJoCo is the Time Master

**QEMU must run as a time slave.** Virtual time does NOT advance on its own.
Instead, the external `TimeAuthority` (running inside the MuJoCo container) sends a
`clock/advance/{delta_ns}` message via **Zenoh** once per `mj_step()`. QEMU advances
exactly `delta_ns` nanoseconds of virtual time and then blocks, waiting for the next
quantum. This guarantees that physics and firmware are always causally consistent —
firmware never runs ahead of or behind the physics simulation.

This is implemented via a **native Zenoh QOM plugin** (`hw/zenoh/zenoh-clock.c`):
- The plugin links `zenoh-c` directly inside QEMU — no Python middleman
- It installs a cooperative hook in QEMU's TCG main loop
- At each quantum boundary the hook blocks the QEMU thread on a Zenoh `get` to
  `sim/clock/advance/{node_id}`; QEMU halts exactly at the translation-block boundary
- TimeAuthority replies with `delta_ns`; the hook resumes the TCG loop
- For sub-quantum precision (`slaved-icount` mode) the plugin additionally manipulates
  `timers_state.qemu_icount_bias`; icount must be enabled in this case

**Implication for all QEMU config**: `slaved-suspend` (no icount, TB-boundary halts) is the
default. `slaved-icount` (`-icount shift=0,align=off,sleep=off`) is reserved for firmware
that measures intervals shorter than one physics quantum.

### FirmwareStudio is a POC — Design is Flexible

FirmwareStudio's current design is a proof of concept. virtmcu can and should drive
design changes there. Flag anything that should change when writing Phase 7 code.

### Recommended FirmwareStudio Design Changes (when Phase 7 arrives)

| Current POC design | Recommended change | Reason |
|---|---|---|
| `apply_patch.py` code-injection approach | `patches/apply_libqemu.py` in virtmcu (done) | Reproducible, version-controlled, reviewable |
| `-icount` + `qemu_icount_bias` as the only clock mode | Native Zenoh QOM plugin with TB-boundary cooperative halt (`slaved-suspend`) as default | ~95% free-run speed; no external Python process |
| IVSHMEM PCI device for all sensor/actuator I/O | QOM peripheral models via arm-generic-fdt | Sensors defined in `.repl`, no hardcoded PCI setup |
| Hardcoded Cortex-A15 machine | `arm-generic-fdt` + `repl2qemu` | Any board from a `.repl` file |
| `node_agent.py` embedded in `cyber/src/` | **Deleted** — replaced by `hw/zenoh/` native plugin | Eliminates Python from the simulation loop entirely |
| `studio_server.py` coupling MCP to QEMU | Keep MCP as the AI/IDE layer; virtmcu exposes QMP | Separation of concerns |
| QEMU 10.2.1 pinned download | virtmcu-patched 11.0.0-rc3 image from `docker/Dockerfile` | Arm-generic-fdt patches, better APIs |

### What FirmwareStudio Currently Uses (to be replaced/improved by virtmcu)

| Current (FirmwareStudio) | Target (virtmcu) |
|---|---|
| Hardcoded Cortex-A15 `-M none` machine | `arm-generic-fdt` machine from .repl |
| IVSHMEM PCI device for sensor/actuator I/O | Proper QOM peripheral models |
| Placeholder `libqemu.c` patch (fake hashes) | Real, tested, upstreamable patch |
| Single machine type per Docker image | Any machine from a `.repl` file |
| Manual QEMU CLI construction | `repl2qemu` generates the CLI automatically |

### Zenoh as the Federation Bus

The message bus is **Eclipse Zenoh**. The native Zenoh QOM plugin (`hw/zenoh/`) links
`zenoh-c` directly; Python Zenoh (`eclipse-zenoh`) is used only by the TimeAuthority
(which stays Python in MuJoCo) and by test tooling. `requirements.txt` includes
`eclipse-zenoh` for test harness use.

Key topics:
- `sim/clock/advance/{node_id}` — TimeAuthority → hw/zenoh/zenoh-clock.c: advance by N ns
- `sim/eth/frame/{node_id}/tx` — QEMU hw/zenoh/ → coordinator: outbound Ethernet frame
- `sim/eth/frame/{node_id}/rx` — coordinator → QEMU hw/zenoh/: inbound frame + delivery vtime
- `firmware/state` — optional telemetry to UI/API (out of scope for Phases 1-6)

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

**What we adopt from qbox**: The cooperative suspend/resume approach for `slaved-suspend`
mode — specifically the pattern of hooking into the TCG loop at quantum boundaries and
blocking the QEMU thread until the external scheduler grants permission to advance.
Unlike qbox (which uses in-process SystemC), our scheduler message arrives via Zenoh,
so the hook is in a native C module (`hw/zenoh/zenoh-clock.c`) that blocks on a Zenoh
queryable rather than a SystemC event. This gives the same TB-boundary precision without
the SystemC dependency.

**What we skip**: External Python QMP stop/cont (too much IPC jitter) and full
SystemC/TLM-2.0 embedding. Zenoh is language-agnostic, works across containers, and is
already part of FirmwareStudio's infrastructure.

### Phase 7 (planned) — FirmwareStudio Integration

Phase 7 will:
1. Write `hw/zenoh/zenoh-clock.c` — native QOM device that links `zenoh-c`, hooks the
   TCG loop, and implements cooperative suspend/resume at quantum boundaries
2. Write `hw/zenoh/zenoh-netdev.c` — custom `-netdev` backend that publishes/subscribes
   Ethernet frames over Zenoh with virtual timestamps for deterministic multi-node delivery
3. Delete `tools/node_agent/` (replaced by the native plugin above)
4. Validate end-to-end: MuJoCo step → TimeAuthority → Zenoh → hw/zenoh/ plugin → TCG halt
   → sensor update → resume → firmware response → physics

---

## Commit / PR Conventions

- Branch: `feature/<phase>-<short-description>`
- Commit format: `<scope>: <imperative description>` (e.g., `hw/dummy: add minimal QOM SysBusDevice`)
- One logical change per commit. Do not mix build system changes with C code changes.
