# qenode

**Make QEMU behave like Renode** — dynamic device loading, FDT-based ARM machine
instantiation, platform description parsing, and Robot Framework test parity.

Part of the **FirmwareStudio** digital twin platform, where MuJoCo physics drives
the simulation clock and QEMU runs firmware in lockstep with the physical world.

---

## The Problem

[Renode](https://renode.io/) is excellent for embedded systems testing: hardware
descriptions live in text files, peripherals hot-plug without recompilation, and the
Robot Framework integration is first-class. QEMU is faster and more widely adopted,
but traditionally requires recompiling the emulator to add devices and uses hardcoded C
structs for machine definitions.

This project closes that gap.

---

## Architecture in One Paragraph

We run **QEMU 11.0.0-rc2** augmented with the **arm-generic-fdt** patch series, which
adds a new ARM machine type that instantiates CPUs, memory, and peripherals entirely
from a Device Tree blob at runtime. Our **`repl2qemu`** Python tool compiles Renode
`.repl` platform files into that Device Tree. Custom peripheral models are compiled as
**shared libraries** from C (or Rust), integrated into QEMU's Meson build via a source
symlink so `-device mydevice` discovers them automatically. A **QMP-backed Robot
Framework library** replaces Renode's test keywords. When running inside FirmwareStudio,
QEMU advances virtual time only when MuJoCo grants it a clock quantum — keeping firmware
and physics causally consistent.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full technical deep-dive,
including the timing design, prior art (qbox, MINRES), and SystemC integration paths.

---

## Repository Layout

```
qenode/
├── CLAUDE.md                   # AI agent context: all constraints and decisions
├── PLAN.md                     # Phased task checklist — check here for status
├── CONTRIBUTING.md             # Setup, dev workflow, code style
│
├── hw/                         # C peripheral models (native QOM only — no Python)
│   ├── dummy/dummy.c           # Minimal QOM SysBusDevice — start here
│   ├── zenoh/                  # [Phase 7] Native Zenoh QOM plugin
│   │   ├── zenoh-clock.c       # TCG cooperative halt + Zenoh clock sync
│   │   └── zenoh-netdev.c      # Custom -netdev backend for deterministic multi-node networking
│   └── meson.build             # Integrates hw/ into QEMU's module build
│
├── tools/
│   ├── node_agent/             # DEPRECATED — superseded by hw/zenoh/ in Phase 7
│   │   └── ...                 # Kept as wire-protocol reference only
│   ├── repl2qemu/              # Renode .repl → Device Tree + QEMU CLI (offline Python tool)
│   └── testing/
│       ├── qemu_keywords.robot # Robot Framework compatibility layer (Renode keyword parity)
│       ├── test_qmp.py         # pytest primary test suite (recommended for new tests)
│       └── qmp_bridge.py       # Async QMP helper
│
├── patches/
│   ├── arm-generic-fdt-v3.mbx  # 33-patch series fetched via b4 (apply with git am)
│   └── apply_libqemu.py        # Phase 1-6 stepping stone: injects icount bias socket
│                               # Superseded by hw/zenoh/zenoh-clock.c in Phase 7
│
├── scripts/
│   ├── setup-qemu.sh           # Clone QEMU, apply patches, symlink hw/, build
│   └── run.sh                  # Launch wrapper: sets QEMU_MODULE_DIR
│
├── docker/
│   ├── Dockerfile              # Multi-stage build: patched QEMU + Python tools
│   └── docker-compose.yml      # Standalone test environment (Zenoh + cyber-node)
│
├── docs/
│   └── ARCHITECTURE.md         # Deep-dive: comparisons, pillars, timing, prior art, SystemC
│
├── tutorial/
│   ├── lesson1-dynamic-machines/ # Educational content for Phase 1
│   ├── lesson2-dynamic-plugins/  # Educational content for Phase 2
│   └── lesson3-repl2qemu/        # Educational content for Phase 3
│
├── Makefile                    # make setup / build / run / test-integration / venv / test
└── requirements.txt            # qemu.qmp, robotframework, lark, eclipse-zenoh
```

---

## Where to Start

**Understanding the project**: Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
Sections 2–3 cover QEMU vs Renode and the four implementation pillars.
Section 7 covers the MuJoCo time master design. Section 8 covers prior art.

**Tutorials**: If you want to understand how QEMU builds the machine dynamically from a Device Tree, start with `tutorial/lesson1-dynamic-machines/README.md`.

**Writing a new peripheral**: Copy `hw/dummy/dummy.c`, rename, implement MMIO ops.
Add an entry in `hw/meson.build`. Run `make build` then:
```bash
./scripts/run.sh -device your-device-name -M arm-generic-fdt -nographic
```

**Running the repl2qemu tool**:
You can use the translator directly, or simply pass your `.repl` file to the run script.
```bash
source .venv/bin/activate
# Native: boots by auto-translating the REPL to a Device Tree
./scripts/run.sh --repl test/phase3/test_board.repl --kernel test/phase1/hello.elf -nographic
```

**Using Standard Device Trees**:
If you aren't using Renode, you can pass standard Device Tree Source (`.dts`) or Blobs (`.dtb`) directly.
```bash
# Auto-compiles the .dts to .dtb and boots
./scripts/run.sh --dts test/phase1/minimal.dts --kernel test/phase1/hello.elf -nographic

# Loads a pre-compiled blob directly
./scripts/run.sh --dtb test/phase1/minimal.dtb --kernel test/phase1/hello.elf -nographic
```

**Probing the Object Model (Debugging)**:
You can interactively dump and inspect the running QEMU Object Model (QOM) tree to verify your Device Trees and Plugins.
First, launch QEMU with the QMP socket enabled:
```bash
./scripts/run.sh -M arm-generic-fdt ... -qmp unix:qmp.sock,server,nowait
```
Then, use the probe tool:
```bash
python3 tools/qmp_probe.py tree
python3 tools/qmp_probe.py get /machine/peripheral-anon/device[0] realized
```

**Running with FirmwareStudio** (external clock, Phase 7+):

The clock synchronization is handled by the native Zenoh QOM plugin (`hw/zenoh/`) which
compiles into QEMU. No separate Python agent process is needed. The TimeAuthority (running
in the MuJoCo container) connects directly to QEMU over Zenoh:
```bash
# slaved-suspend (default — full TCG speed, ~95% throughput)
./scripts/run.sh -M arm-generic-fdt -hw-dtb board.dtb -kernel firmware.elf \
    -device zenoh-clock,node=0,router=tcp/localhost:7447

# slaved-icount (exact ns precision — only for sub-quantum hardware timer firmware)
./scripts/run.sh -M arm-generic-fdt -hw-dtb board.dtb -kernel firmware.elf \
    -device zenoh-clock,node=0,router=tcp/localhost:7447,mode=icount \
    -icount shift=0,align=off,sleep=off
```

**Docker (CI or Phase 4+ with TCG plugins)**:
```bash
docker compose -f docker/docker-compose.yml up
```

---

## Prerequisites

**macOS and Linux** are both supported for development. Windows is not.

On macOS, native builds work for Phases 1–3. For Phase 4+ (TCG plugins for coverage and
tracing), use Docker — macOS has a conflict between `--enable-modules` and
`--enable-plugins` that breaks module loading (GitLab #516). See
`docs/ARCHITECTURE.md §6` for the full decision table.

```bash
# macOS (Homebrew)
brew install ninja meson dtc pkg-config glib pixman b4

# Linux (Debian/Ubuntu)
sudo apt install build-essential libglib2.0-dev ninja-build python3-venv \
                 device-tree-compiler flex bison libpixman-1-dev pkg-config b4

# All platforms
make setup        # clone QEMU, apply patches, build (~5 min first run)
make venv         # create .venv and install Python deps
source .venv/bin/activate
make run          # smoke-test
```

---

## Current Status

See [`PLAN.md`](PLAN.md) for the full phased checklist.

| Phase | Description | Status |
|-------|-------------|--------|
| 0 | Repository setup, documentation | **Done** |
| 1 | QEMU build with arm-generic-fdt patches | **Done** |
| 2 | Dynamic QOM plugin infrastructure | **Done** |
| 3 | repl2qemu parser (.repl → .dtb + QEMU CLI) | **Done** |
| 3.5 | YAML Platform & OpenUSD Alignment | **Done** |
| 4 | Robot Framework QMP library | Not started |
| 5 | Co-simulation bridge (Verilated from Renode / SystemC) | Deferred |
| 6 | Multi-node wireless medium coordinator | Future |
| 7 | FirmwareStudio / MuJoCo external time master | Future |

---

## Key Technical Decisions

- **Meson integration, not LD_PRELOAD**: `hw/` is symlinked into QEMU's source tree so
  devices compile as proper QEMU modules with auto-discovery. `-device foo` just works.
- **No Python in the simulation loop**: All peripherals, clock sync, and networking are
  native C/Rust QOM modules. Python is offline-only (repl2qemu, pytest). See ADR-003.
- **Three clock modes**: `standalone` (full speed; KVM/hvf available for Cortex-A on ARM
  hosts), `slaved-suspend` (native TCG hook, ~95% speed, recommended for FirmwareStudio),
  `slaved-icount` (exact ns, ~15% speed, sub-quantum timers only). Implemented as
  `hw/zenoh/zenoh-clock.c`. KVM/hvf prohibited in slaved modes and for Cortex-M. See
  ADR-001 and ADR-009.
- **Deterministic multi-node**: `hw/zenoh/zenoh-netdev.c` with virtual-timestamped frames,
  not UDP multicast. See ADR-002.
- **`query-cpus-fast`**: The old `query-cpus` QMP command is deprecated.
- **arm-generic-fdt is not upstream**: 33-patch patchew series on QEMU 11.0.0-rc2.
- **SystemC peripherals**: Path B (Remote Port, Phase 5) is primary. Path A requires
  writing `hw/misc/mmio-socket-bridge.c` first — QEMU does not natively serialize MMIO
  to sockets. See ADR-005 and `docs/ARCHITECTURE.md §9`.

---

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Branch: `feature/<phase>-<short-desc>`.
Commit style: `scope: imperative description` (e.g., `hw/uart: add pl011 read stub`).
