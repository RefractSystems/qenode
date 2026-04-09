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
├── hw/                         # C peripheral models
│   ├── dummy/dummy.c           # Minimal QOM SysBusDevice — start here
│   └── meson.build             # Integrates hw/ into QEMU's module build
│
├── tools/
│   ├── node_agent/             # Bridges Zenoh (MuJoCo TimeAuthority) ↔ QEMU clock socket
│   │   ├── qemu_clock.py       # Async client for the libqemu wire protocol
│   │   └── __main__.py         # Entry point; supports standalone / slaved-suspend / slaved-icount
│   ├── repl2qemu/              # Renode .repl → Device Tree + QEMU CLI
│   └── testing/
│       ├── qemu_keywords.robot # Robot Framework resource (Renode keyword replacements)
│       └── qmp_bridge.py       # Async QMP helper
│
├── patches/
│   ├── arm-generic-fdt-v3.mbx  # 33-patch series fetched via b4 (apply with git am)
│   └── apply_libqemu.py        # Injects the -clocksock external time master extension
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
├── Makefile                    # make setup / build / run / venv / test
└── requirements.txt            # qemu.qmp, robotframework, lark, eclipse-zenoh
```

---

## Where to Start

**Understanding the project**: Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
Sections 2–3 cover QEMU vs Renode and the four implementation pillars.
Section 7 covers the MuJoCo time master design. Section 8 covers prior art.

**Writing a new peripheral**: Copy `hw/dummy/dummy.c`, rename, implement MMIO ops.
Add an entry in `hw/meson.build`. Run `make build` then:
```bash
./scripts/run.sh -device your-device-name -M arm-generic-fdt -nographic
```

**Running the repl2qemu tool**:
```bash
source .venv/bin/activate
python -m tools.repl2qemu path/to/board.repl --out-dtb board.dtb --print-cmd
```

**Running with FirmwareStudio** (external clock):
```bash
CLOCK_MODE=slaved-suspend NODE_ID=0 python -m tools.node_agent &
./scripts/run.sh -M arm-generic-fdt -hw-dtb board.dtb -kernel firmware.elf \
    -clocksock /tmp/qemu-clock.sock -icount shift=0,align=off,sleep=off
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
| 1 | QEMU build with arm-generic-fdt patches | In progress |
| 2 | Dynamic QOM plugin infrastructure | Not started |
| 3 | repl2qemu parser (.repl → .dtb + QEMU CLI) | Not started |
| 4 | Robot Framework QMP library | Not started |
| 5 | Co-simulation bridge (Verilated from Renode / SystemC) | Deferred |
| 6 | Multi-node wireless medium coordinator | Future |
| 7 | FirmwareStudio / MuJoCo external time master | Future |

---

## Key Technical Decisions

- **Meson integration, not LD_PRELOAD**: `hw/` is symlinked into QEMU's source tree so
  devices compile as proper QEMU modules with auto-discovery. `-device foo` just works.
- **Three clock modes**: `standalone` (full speed), `slaved-suspend` (QMP stop/cont,
  ~95% speed, recommended for FirmwareStudio), `slaved-icount` (exact ns, ~15% speed,
  only for sub-quantum timer precision). See `docs/ARCHITECTURE.md §5`.
- **`query-cpus-fast`**: The old `query-cpus` QMP command is deprecated. All QMP code
  uses `query-cpus-fast`.
- **arm-generic-fdt is not upstream**: 33-patch patchew series on QEMU 11.0.0-rc2.
- **vhost-user is VirtIO-specific**: Cannot back arbitrary MMIO peripherals without a
  VirtIO transport in the guest.
- **SystemC peripherals**: Three integration paths — chardev socket adapter (now),
  Remote Port (Phase 5), qbox (future). See `docs/ARCHITECTURE.md §9`.

---

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Branch: `feature/<phase>-<short-desc>`.
Commit style: `scope: imperative description` (e.g., `hw/uart: add pl011 read stub`).
