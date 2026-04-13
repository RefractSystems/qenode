# virtmcu

**A deterministic multi-node firmware simulation framework** built on QEMU, designed for
cyber-physical digital twin development. virtmcu makes multiple QEMU instances behave as
a coordinated network of microcontrollers — time-slaved to a physics engine, communicating
over a deterministic message bus, with sensor and actuator I/O abstracted through a
well-defined hardware boundary.

Part of the **FirmwareStudio** digital twin platform, where MuJoCo physics drives the
simulation clock and QEMU runs firmware in lockstep with the physical world.

---

## What Problem This Solves

Embedded firmware for cyber-physical systems (drones, robots, motor controllers) does not
run in isolation. It interacts with sensors, actuators, and other microcontrollers — all
governed by physics that unfolds in continuous time. Testing this firmware in simulation
requires three things that stock QEMU does not provide:

1. **Deterministic virtual time across nodes.** When firmware on MCU-A reads a sensor and
   sends a CAN frame to MCU-B, the delivery time must be a function of virtual time, not
   wall-clock scheduling jitter. Otherwise test results are not reproducible.

2. **A clean sensor/actuator boundary.** Peripherals must translate between the binary
   register world of firmware and the continuous physical properties (force, acceleration,
   angle) of a physics simulation. There is no standard QEMU mechanism for this.

3. **External clock authority.** QEMU's virtual clock must not free-run. It must advance
   only when the physics engine grants a time quantum, so that firmware never runs ahead
   of or behind the simulated physical world.

virtmcu addresses all three at the QEMU layer, using native C/Rust QOM modules and Zenoh
as the inter-node message bus. No Python daemons in the simulation loop; no approximations
in inter-node timing.

---

## Architecture in One Paragraph

**QEMU 11.0.0-rc3**, augmented with the **arm-generic-fdt** patch series and native **RISC-V virt** capabilities, instantiates
ARM and RISC-V hardware entirely from a Device Tree blob at runtime. Custom peripheral models compile
as **shared libraries** and are auto-discovered via QEMU's module system — no `LD_PRELOAD`,
no recompilation of the emulator. A native **Zenoh QOM plugin** (`hw/zenoh/`) links
`zenoh-c` directly into QEMU: it hooks the TCG execution loop at translation-block
boundaries to implement cooperative suspend/resume, acts as a deterministic Ethernet and
UART backend for multi-node communication, and synchronizes virtual time with the external
physics engine. A **Sensor/Actuator Abstraction Layer (SAL/AAL)** translates raw MMIO
registers into physical quantities, with two modes: standalone CI/CD (replay from Renode
Sensor Data files) and integrated (lock-step with MuJoCo via shared memory). A
**QMP-backed Robot Framework library** provides test automation parity with Renode's
keyword suite.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full technical deep-dive.

---

## Core Capabilities

- **Dynamic ARM and RISC-V Machines**: Instantiate boards from a Device Tree (`.dtb`) at
  runtime using the `arm-generic-fdt` or `virt` machine types. No hardcoded C machine structs.

- **Dynamic QOM Plugins**: Write peripherals in C or Rust, compile to `.so`, load with
  `-device` — no QEMU recompilation required. Discovered automatically via QEMU's module
  system (`--enable-modules`).

- **Deterministic Multi-Node Networking**: `hw/zenoh/zenoh-netdev.c` delivers Ethernet
  frames between QEMU instances with embedded virtual timestamps. Frames are buffered and
  injected into the guest NIC only when virtual time reaches the stamped arrival time.
  No UDP multicast jitter.

- **Deterministic Multi-Node UART**: `hw/zenoh/zenoh-chardev.c` extends the same
  virtual-timestamp model to serial ports, enabling multi-node UART communication and
  human-in-the-loop interactivity with correct virtual ordering.

- **Cooperative Time Slaving**: `hw/zenoh/zenoh-clock.c` blocks QEMU's TCG loop at each
  quantum boundary, waiting for a Zenoh `GET` reply from the external TimeAuthority.
  Two modes: `slaved-suspend` (full TCG speed, ~95% throughput — default) and
  `slaved-icount` (exact nanosecond virtual time — for firmware measuring sub-quantum
  intervals such as PWM or µs-precision DMA).

- **Sensor/Actuator Abstraction (SAL/AAL)**: Peripheral models translate raw firmware
  register writes/reads into continuous physical properties (floating-point force,
  acceleration, angle) with configurable noise and transfer functions.
  - *Standalone mode*: Ingest Renode Sensor Data (RESD) binary files for fast, fully
    deterministic CI/CD replay without a physics engine.
  - *Integrated mode*: Lock-step with MuJoCo (zero-copy `mjData` shared memory) or
    NVIDIA Omniverse (Accellera Federated Simulation Standard / OpenUSD schemas).

- **Co-Simulation Bridge**: Connect Verilated C++ models or FPGAs via SystemC TLM-2.0
  and Remote Port sockets. Phase 9 extends this to shared physical media (CAN, SPI) with
  asynchronous IRQ support.

- **Platform Description Tools**: `repl2qemu` compiles legacy Renode `.repl` files or
  OpenUSD-aligned `.yaml` board descriptions into Device Tree blobs and QEMU CLI strings.

- **Unified Test Automation**: pytest + `qemu.qmp` for primary test suites; a Robot
  Framework compatibility layer for Renode `.robot` suite migration.

---

## Repository Layout

```
virtmcu/
├── CLAUDE.md                   # AI agent context: constraints and architecture decisions
├── CONTRIBUTING.md             # Dev workflow, code style, setup
│
├── hw/                         # C/Rust QOM peripheral models (no Python in sim loop)
│   ├── dummy/dummy.c           # Minimal QOM SysBusDevice — start here (C)
│   ├── rust-dummy/             # Minimal QOM SysBusDevice — start here (Rust FFI)
│   ├── misc/
│   │   └── mmio-socket-bridge.c # Legacy custom socket bridge
│   ├── remote-port/            # AMD/Xilinx Remote Port QOM bridge for SystemC/Verilator
│   ├── zenoh/                  # Native Zenoh QOM plugin
│   │   ├── zenoh-clock.c       # TCG cooperative halt + Zenoh clock sync
│   │   ├── zenoh-netdev.c      # Deterministic multi-node Ethernet backend
│   │   └── zenoh-chardev.c     # Deterministic multi-node UART backend
│   └── meson.build             # Integrates hw/ into QEMU's module build
│
├── tools/
│   ├── repl2qemu/              # .repl / .yaml → Device Tree + QEMU CLI (offline)
│   ├── systemc_adapter/        # C++ SystemC TLM-2.0 ↔ Remote Port / Custom Socket bridge
│   ├── cyber_bridge/           # C++ SAL/AAL telemetry and MuJoCo shm synchronization
│   ├── zenoh_coordinator/      # Rust daemon for strictly ordering multi-node frames
│   └── testing/
│       ├── qemu_keywords.robot # Robot Framework compatibility layer
│       ├── test_qmp.py         # pytest primary test suite
│       └── qmp_bridge.py       # Async QMP helper
│
├── patches/
│   ├── arm-generic-fdt-v3.mbx  # 33-patch series (apply with git am)
│   ├── apply_zenoh_hook.py     # Injects virtmcu_tcg_quantum_hook into cpu-exec.c
│   └── apply_zenoh_netdev.py   # Injects Zenoh netdev backend registration
│
├── scripts/
│   ├── setup-qemu.sh           # Clone QEMU, apply patches, symlink hw/, build
│   └── run.sh                  # Launch wrapper: sets QEMU_MODULE_DIR, detects arch
│
├── docker/
│   ├── Dockerfile              # Multi-stage: toolchain / devenv / builder / runtime
│   └── docker-compose.yml      # Standalone test environment
│
├── test/                       # End-to-end integration and smoke tests per subsystem
│
└── docs/
    ├── ARCHITECTURE.md         # Deep-dive: design pillars, timing, prior art, ADRs
    └── TIME_MANAGEMENT.md      # Detailed guide to BQL mechanics and physics sync
```

---

## Where to Start

**Read the architecture first**: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
Sections 1–3 cover the design rationale and the five implementation pillars. Section 5
covers the timing design and BQL constraints. Section 6 covers prior art (qbox, MINRES).

**For a deep dive on clock modes and BQL mechanics**: [`docs/TIME_MANAGEMENT.md`](docs/TIME_MANAGEMENT.md).

**Write a new peripheral**: Copy `hw/dummy/dummy.c`. Rename, implement MMIO ops, add an
entry in `hw/meson.build`. Run `make build` then:
```bash
./scripts/run.sh --dtb test/phase1/minimal.dtb -device your-device-name -nographic
```

**Run the repl2qemu tool**:
```bash
source .venv/bin/activate
./scripts/run.sh --repl test/phase3/test_board.repl --kernel test/phase1/hello.elf -nographic
```

**Run with FirmwareStudio** (external time master, Phase 7+):
```bash
# slaved-suspend (default — full TCG speed, ~95% throughput)
./scripts/run.sh --dtb board.dtb --kernel firmware.elf \
    -device zenoh-clock,node=0,router=tcp/localhost:7447

# slaved-icount (exact ns — only for sub-quantum hardware timer firmware)
./scripts/run.sh --dtb board.dtb --kernel firmware.elf \
    -device zenoh-clock,node=0,router=tcp/localhost:7447,mode=icount \
    -icount shift=0,align=off,sleep=off
```

**Docker** (CI or Phase 4+ with TCG plugins):
```bash
docker compose -f docker/docker-compose.yml up
```

---

## Setup

**macOS and Linux** are supported. Windows is not.

### Dev Container (recommended)

Open in VS Code and accept **"Reopen in Container"**. Everything runs automatically.

### Manual

```bash
# macOS (Homebrew)
brew install ninja meson dtc pkg-config glib pixman b4

# Linux (Debian/Ubuntu)
sudo apt install build-essential libglib2.0-dev ninja-build python3-venv \
                 device-tree-compiler flex bison libpixman-1-dev pkg-config b4

# All platforms
git submodule update --init --recursive   # fetch QEMU source
make setup        # apply patches, build QEMU (~10 min first run)
make venv         # create .venv and install Python deps
source .venv/bin/activate
make run          # smoke-test
```

> **macOS note**: Native builds work for Phases 1–3. Phase 4+ requires Docker — a
> GLib conflict (`--enable-modules` + `--enable-plugins`, GitLab #516) breaks module
> loading on macOS. See `docs/ARCHITECTURE.md §6`.

---

## Current Status

The core framework development is complete. All architectural pillars and capabilities listed above have been implemented, tested, and integrated into the CI/CD pipeline.

- [x] Dynamic ARM and RISC-V machine generation from `.repl` and `.yaml` files.
- [x] Dynamic QOM plugin infrastructure for C and Rust peripherals.
- [x] Native Zenoh clock plugin (`slaved-suspend` and `slaved-icount` modes) for physics engine synchronization.
- [x] Deterministic multi-node Ethernet and UART communication via Zenoh.
- [x] Sensor/Actuator Abstraction Layers (SAL/AAL) for MuJoCo and OpenUSD integration.
- [x] Full TLM-2.0 co-simulation via AMD/Xilinx Remote Port and SystemC.
- [x] Automated test suite using pytest, QMP, and Robot Framework keywords.

---

## Key Design Decisions

- **No Python in the simulation loop.** All peripherals, clock sync, and networking are
  native C/Rust QOM modules. Python is offline-only (repl2qemu, pytest). See ADR-003.
- **Zenoh as the federation bus.** A single message bus handles clock quanta, Ethernet
  frames, UART bytes, and sensor data. Language-agnostic, works across containers.
- **Three clock modes.** `standalone` (free-run, full speed), `slaved-suspend` (~95%
  throughput — recommended default for FirmwareStudio), `slaved-icount` (exact nanosecond
  virtual time — for sub-quantum hardware timers). Implemented in `hw/zenoh/zenoh-clock.c`.
- **Meson integration, not LD_PRELOAD.** `hw/` is symlinked into QEMU's source tree so
  devices compile as proper QEMU modules with auto-discovery. `-device foo` just works.
- **arm-generic-fdt is not upstream.** 33-patch patchew series on QEMU 11.0.0-rc3.
- **Virtual-timestamped delivery.** Multi-node packets and UART bytes carry virtual
  timestamps and are delivered to the guest NIC or chardev only when virtual time catches
  up. Deterministic by construction, not by coordination.

---

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Branch: `feature/<phase>-<short-desc>`.
Commit style: `scope: imperative description` (e.g., `hw/zenoh: add chardev backend`).
