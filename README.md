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

**QEMU 11.0.0**, augmented with the **arm-generic-fdt** patch series and native **RISC-V virt** capabilities, instantiates
ARM and RISC-V hardware entirely from a Device Tree blob at runtime. Custom peripheral models compile
as **shared libraries** and are auto-discovered via QEMU's module system — no `LD_PRELOAD`,
no recompilation of the emulator. A native **Zenoh QOM plugin** (`hw/rust/`) links
the official `zenoh` Rust crate directly into QEMU: it hooks the TCG execution loop at translation-block
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

- **Deterministic Multi-Node Networking**: `hw/rust/zenoh-netdev` delivers Ethernet
  frames between QEMU instances with embedded virtual timestamps. Frames are buffered and
  injected into the guest NIC only when virtual time reaches the stamped arrival time.
  No UDP multicast jitter.

- **Deterministic Multi-Node UART**: `hw/rust/zenoh-chardev` extends the same
  virtual-timestamp model to serial ports, enabling multi-node UART communication and
  human-in-the-loop interactivity with correct virtual ordering.

- **Cooperative Time Slaving**: `hw/rust/zenoh-clock` blocks QEMU's TCG loop at each
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
  Framework compatibility layer for Renode `.robot` suite migration. Tests run in parallel
  by default to ensure rapid feedback.

---

## Repository Layout

```
virtmcu/
├── CLAUDE.md                   # AI agent context: constraints and architecture decisions
├── CONTRIBUTING.md             # Dev workflow, code style, setup
│
├── hw/                         # Rust QOM peripheral models (no Python in sim loop)
│   ├── remote-port/            # AMD/Xilinx Remote Port QOM bridge for SystemC/Verilator
│   ├── rust/                   # All native Rust plugins and bridges
│   │   ├── rust-dummy/         # Minimal Rust QOM SysBusDevice — start here
│   │   ├── zenoh-clock/        # TCG cooperative halt + Zenoh clock sync
│   │   ├── zenoh-netdev/       # Deterministic multi-node Ethernet backend
│   │   └── zenoh-chardev/      # Deterministic multi-node UART backend
│   └── meson.build             # Integrates hw/ into QEMU's module build
│
├── tools/                      # Assorted offline utilities and debugging helpers
│   ├── yaml2qemu.py            # .yaml → Device Tree + QEMU CLI transpiler
│   ├── cyber_bridge/           # C++ SAL/AAL telemetry and MuJoCo shm synchronization
│   ├── debug/                  # Python GDB helpers for interactive debugging
│   └── zenoh_coordinator/      # Rust daemon for strictly ordering multi-node frames
│
├── tests/                      # pytest / Robot test suites
│   ├── conftest.py             # pytest fixtures for Zenoh and QEMU orchestration
│   ├── test_qmp_keywords.robot # Robot Framework integration
│   └── ...                     # Extensive coverage tests (phase1, phase2, etc.)
│
├── patches/
│   ├── arm-generic-fdt-v3.mbx  # 33-patch series (applied by setup-qemu.sh)
│   ├── apply_zenoh_hook.py     # Injects virtmcu_tcg_quantum_hook into cpu-exec.c
│   └── ...
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
    └── TIME_MANAGEMENT_DESIGN.md # Detailed guide to BQL mechanics and physics sync
```

---

## Where to Start

**Read the architecture first**: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
Sections 1–3 cover the design rationale and the five implementation pillars. Section 5
covers the timing design and BQL constraints. Section 6 covers prior art (qbox, MINRES).

**For a deep dive on clock modes and BQL mechanics**: [`docs/TIME_MANAGEMENT_DESIGN.md`](docs/TIME_MANAGEMENT_DESIGN.md).

**Write a new peripheral**: Navigate to `hw/rust/rust-dummy/` as a template. Rename, implement MMIO ops, and add an
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

Open in VS Code and accept **"Reopen in Container"**. Everything runs automatically. For detailed instructions on setting up SSH/Git credentials inside the container, see [**docs/CONTAINER_DEV_GUIDE.md**](docs/CONTAINER_DEV_GUIDE.md).

### Manual (Native Linux Only)

> **⚠️ macOS & Windows Support:** virtmcu requires a Linux environment. macOS and Windows developers **MUST** use the provided Devcontainer. Bare-metal development on macOS/Windows is not supported. For macOS developers, we highly recommend using [OrbStack](https://orbstack.dev/) instead of Docker Desktop for a significantly faster and more native Devcontainer experience.

```bash
# Linux (Debian/Ubuntu)
sudo apt install build-essential libglib2.0-dev ninja-build \
                 device-tree-compiler flex bison libpixman-1-dev pkg-config b4
curl -LsSf https://astral.sh/uv/install.sh | sh

# All platforms
make setup-initial # clones QEMU, applies patches, and builds (~10 min first run)
make venv          # synchronize Python environment with uv
source .venv/bin/activate
make run           # smoke-test
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
  virtual time — for sub-quantum hardware timers). Implemented in `hw/rust/zenoh-clock`.
- **Meson integration, not LD_PRELOAD.** `hw/` is symlinked into QEMU's source tree so
  devices compile as proper QEMU modules with auto-discovery. `-device foo` just works.
- **arm-generic-fdt is not upstream.** 33-patch patchew series on QEMU 11.0.0.
- **Virtual-timestamped delivery.** Multi-node packets and UART bytes carry virtual
  timestamps and are delivered to the guest NIC or chardev only when virtual time catches
  up. Deterministic by construction, not by coordination.

---

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Branch: `feature/<phase>-<short-desc>`.
Commit style: `scope: imperative description` (e.g., `hw/rust/zenoh-chardev: add buffering`).

---

## AI-Driven Development (Auto Green)

This repository is optimized for AI-driven development. If the CI/CD builds are failing, you can use the **"Auto Green"** feature by prompting your AI agent (Claude Code or Gemini CLI) with a simple command:

> **"Fix CI"** or **"Make the builds green"**

When you issue this command, the agent will:
1.  **Diagnose** the failure using `gh` (GitHub CLI) to avoid rate limits.
2.  **Reproduce** the failure locally.
3.  **Align Tests**: If it passes locally but fails remotely, it will update the local test environment to match CI.
4.  **Fix & Verify** the code locally.
5.  **Push & Monitor**: It will push the fix and keep monitoring the remote run until it is officially green.

This ensures that "it works on my machine" issues are caught and fixed at the test-automation layer.
