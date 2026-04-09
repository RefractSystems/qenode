# Architecture: Renode Functionality on a QEMU-Based Framework

## 1. Introduction

Two dominant embedded emulation platforms exist for firmware development and CI:

**QEMU** — C-based, TCG JIT execution, deeply integrated with Linux tooling. Fast and
widely adopted. Traditionally requires recompiling the emulator to add new devices; machine
definitions are hardcoded C structs.

**Renode** — C#-based framework by Antmicro. Human-readable `.repl` platform description
files, hot-pluggable peripherals, deterministic virtual time, and first-class Robot
Framework integration. More ergonomic for rapid peripheral prototyping; slower due to the
C→C# boundary on every MMIO access.

**This project** builds a framework that wraps and extends QEMU to provide Renode's
flexibility while retaining QEMU's performance and ecosystem. It is *not* a fork; it
works alongside an unmodified (or minimally patched) QEMU binary.

---

## 2. Architectural Comparison

### 2.1 Execution Engines

| | QEMU | Renode |
|---|---|---|
| Core engine | TCG (C JIT), optionally KVM | tlib (C, derived from early QEMU) |
| Peripheral access | Direct C function call on MMIO hit | C → C# boundary crossing |
| Determinism | `icount` mode (approximate) | Nanosecond virtual clock, fully deterministic |
| KVM support | Yes (x86, ARM) | Yes (x86 only, recent) |

QEMU's pure-C peripheral path is significantly lower latency than Renode's C→C# boundary.
The trade-off: Renode's C# layer enables dynamic peripheral loading without recompilation.

### 2.2 Device Model

**QEMU — QOM (QEMU Object Model)**:
- All devices are registered as `TypeInfo` structs
- Single inheritance, multiple-interface inheritance
- Device lifecycle: `object_initialize()` → set properties → `realize()`
- Historically: adding a device requires recompiling QEMU
- With `--enable-modules`: devices can be compiled as `.so` files and loaded at runtime,
  *but* the module must be registered in QEMU's compile-time `module_info` table to be
  discoverable via `-device`

**Renode — C# reflection**:
- Devices implement typed interfaces (`IDoubleWordPeripheral`, etc.)
- Missing access widths handled gracefully (returns 0 with warning)
- External C#, Python, or Rust extensions load at runtime without recompiling
- Registers managed by a `RegistersCollection` with rich introspection

### 2.3 Machine / Platform Description

**QEMU**:
- Machines: hardcoded C structs in `hw/<arch>/` source files
- Dynamic via FDT: the `virt` machine (ARM, RISC-V) reads a `.dtb` for memory layout,
  but device drivers for specific peripherals must still be compiled in
- **arm-generic-fdt** (patch series, not yet upstream): a new ARM machine type that
  instantiates any device listed in the Device Tree by matching `compatible` strings to
  registered QOM types — this is the key enabler for this project

**Renode**:
- `.repl` files (YAML-inspired, indented): define peripherals, sysbus addresses, IRQ routing
- `using` keyword for inheritance/composition
- Fully dynamic: no recompilation needed to define a new board

### 2.4 Control and Scripting

**QEMU**:
- HMP (Human Monitor Protocol): interactive CLI, `-monitor stdio`
- QMP (QEMU Machine Protocol): JSON over Unix socket or TCP, used for automation
  - `{"execute": "cont"}` — start/resume
  - `{"execute": "system_reset"}` — hard reset
  - `{"execute": "query-cpus-fast"}` — get CPU state including PC
  - `{"execute": "human-monitor-command", "arguments": {"command-line": "..."}}` — HMP via QMP

**Renode**:
- Monitor / RESC scripts: `mach create`, `machine LoadPlatformDescription`, `sysbus LoadELF`
- Robot Framework integration via `renode-keywords.robot`

### 2.5 Co-Simulation

**Renode**: Verilator integration via `IntegrationLibrary` (`eval()` callback pattern).
EtherBone bridge for FPGA-over-UDP access.

**QEMU**: Remote Port protocol (AMD/Xilinx), SystemC TLM-2.0 interface for
QEMU↔Verilator co-simulation.

### 2.6 Determinism and Multi-Node

**Renode**: Strict virtual time (1 ns resolution), deterministic across runs. Built-in
`WirelessMedium` with distance-based packet loss simulation.

**QEMU**: `icount` mode for approximate instruction-count-based timing. Multi-node via
`-netdev socket` with a coordinator script; non-deterministic without extra work.

---

## 3. Target Architecture (The Four Pillars)

### Pillar 1 — Dynamic QOM Device Loader

Peripheral models are authored in C (or Rust via FFI) and compiled as position-independent
shared objects (`.so` on Linux, `.dylib` on macOS).

**Build integration**: `scripts/setup-qemu.sh` symlinks `qenode/hw/` into QEMU's source
tree as `hw/qenode/` and appends `subdir('qenode')` to `hw/meson.build`. Our
`hw/meson.build` registers devices in QEMU's `modules` dict. With `--enable-modules`,
this produces `hw-qenode-<name>.so` files installed in `QEMU_MODDIR` with correct
`module_info` entries. `-device dummy-device` auto-discovers and loads the `.so` — no
`LD_PRELOAD` hack required, works identically on Linux and macOS.

For Python-implemented peripherals: use vhost-user for VirtIO-class devices (GPIO,
network). For UART/SPI/I2C-class devices: chardev socket daemons. Note: vhost-user is
VirtIO-specific and cannot back arbitrary MMIO peripherals without a VirtIO transport
in the guest firmware.

### Pillar 2 — Platform Description Translation (repl2qemu)

A Python tool (`tools/repl2qemu/`) that:
1. Parses Renode `.repl` files (indent-mode syntax, `using` includes, inline objects)
2. Builds an AST of devices, sysbus addresses, and IRQ routing
3. Emits a DTS file and invokes `dtc` to produce a `.dtb`
4. Generates the equivalent QEMU CLI argument string

The generated `.dtb` is passed to `arm-generic-fdt` via `-hw-dtb`, which instantiates
QOM devices by matching DTS `compatible` strings to registered types — completing the
dynamic machine creation loop.

This is conceptually the reverse of Antmicro's `dts2repl` tool.

### Pillar 3 — Co-Simulation Bridge (Phase 5, deferred)

For projects with Verilated C++ hardware models:
- Replace Renode's `IntegrationLibrary` headers with AMD/Xilinx `libsystemctlm-soc`
- Wrap the Verilated model as a SystemC TLM-2.0 module
- Connect to QEMU via Remote Port Unix sockets
- Remote Port handles time domain synchronization

For EtherBone (FPGA over UDP):
- A custom QOM device (`hw/etherbone/`) intercepts MMIO writes, constructs EtherBone
  packets, and sends them over UDP — mirroring Renode's `EtherBoneBridge`

### Pillar 4 — Unified Test Automation

`tools/testing/qemu_keywords.robot` provides Robot Framework keywords backed by QMP:

| Renode Keyword | QMP Translation |
|---|---|
| `Start Emulation` | `{"execute": "cont"}` |
| `Reset Emulation` | `{"execute": "system_reset"}` |
| `Pause Emulation` | `{"execute": "stop"}` |
| `PC Should Be Equal  ${addr}` | `query-cpus-fast` → assert `pc` |
| `Wait For Line On UART  ${pattern}` | chardev socket readline with regex |
| `Execute Command  ${cmd}` | `human-monitor-command` |

UART output capture: QEMU redirects serial to a Unix socket
(`-chardev socket,id=serial0,path=/tmp/qemu_serial,server=on,wait=off`).
The Robot keyword connects to that socket and polls for pattern matches.

Multi-node (Phase 6): a Python coordinator spawns multiple QEMU instances using
`-netdev socket,mcast=...`, intercepts multicast traffic, and applies attenuation
based on distance to simulate wireless medium behavior.

### Pillar 5 — External Time Master (FirmwareStudio Integration)

qenode is the QEMU layer of **FirmwareStudio**, a digital twin platform where MuJoCo
physics drives the simulation clock. This pillar formalizes the time synchronization
protocol between the physics engine and QEMU.

See Section 7 for the full design and timing analysis.

---

## 4. Migration Phases

### Phase 1: CPU + Memory Baseline
Extract CPU type and memory regions from `.repl` files. Map Renode CPU names to QEMU
targets (`CPU.ARMv7A` → `qemu-system-arm`, `CPU.RiscV32` → `qemu-system-riscv32`).
Boot firmware with `-machine virt -kernel firmware.elf`. Attach GDB (`-s -S`) and verify
execution reaches `Reset_Handler`.

### Phase 2: Dynamic Plugin Infrastructure
Build with `--enable-modules`. Compile `hw/dummy.c` → `modules/hw-dummy.so`. Confirm
LD_PRELOAD injection via `scripts/run.sh`. Validate via `info qom-tree` in QEMU monitor.

### Phase 3: Peripheral Translation (C# → QOM + Python)
For performance-critical peripherals (DMA, MMU, fast timers): translate C# class to C
using QOM `MemoryRegionOps` read/write callbacks.

For low-speed peripherals (I2C sensors, SPI config): implement as Python daemons. Use
chardev socket (`-chardev socket,id=mydev,...`) for simple byte-oriented protocols, or
vhost-user for VirtIO-attached devices.

### Phase 4: repl2qemu Automation
Build the parser. Run against public Renode boards (STM32F4, Zynq). Produce a `.dtb` and
verify `arm-generic-fdt` boots the same firmware that ran on Renode.

### Phase 5: Co-Simulation (deferred)
Migrate Verilated models to SystemC TLM-2.0. Connect via Remote Port. Restore EtherBone
via the custom QOM UDP bridge device.

### Phase 6: Test Automation Parity
Finalize `qemu_keywords.robot`. Run the full legacy Robot Framework suite against QEMU.
Assert identical pass/fail metrics.

---

## 5. Performance Considerations

### QOM Device Performance

- **Pure C QOM devices**: No C→C# boundary overhead. MMIO latency is significantly
  lower than equivalent Renode peripherals.
- **Python daemons via Unix socket**: Each MMIO access crosses a process boundary (~1 µs
  round-trip). Acceptable for peripherals accessed at <1 MHz. Do NOT use for DMA engines,
  fast timers, or interrupt-heavy peripherals.
- **Profiling**: Use Callgrind + QEMU's TCG Continuous Benchmarking to isolate per-device
  MMIO costs.

### External Clock Performance (Three Modes)

When QEMU is slaved to an external time master (e.g., MuJoCo), there are three operating
modes with very different performance profiles:

| Mode | QEMU flags | Throughput | Use when |
|---|---|---|---|
| `standalone` | (none) | **100%** — full TCG speed | Development, CI without physics |
| `slaved-suspend` | `-clocksock <path>` | **~95%** — only quantum-boundary pause | **Recommended default** for FirmwareStudio |
| `slaved-icount` | `-clocksock <path> -icount shift=0,align=off,sleep=off` | **~15–20%** — icount disables TB chaining | Only if firmware measures sub-quantum intervals |

#### slaved-suspend (recommended)

At each physics step boundary the NodeAgent sends QMP `{"execute": "stop"}`, updates
sensor MMIO, then sends `{"execute": "cont"}`. QEMU runs at **full TCG speed** within
each quantum. The only overhead is the ~50 µs Zenoh + QMP round-trip at boundaries.

This is the pattern used by Qualcomm's **qbox** project via its `libgssync` library
(see Section 8). It gives essentially free-run performance for control loops at 1–10 kHz.

#### slaved-icount (when required)

The `libqemu` patch (`patches/apply_libqemu.py`) exposes a `-clocksock` Unix socket.
The NodeAgent sends `ClockAdvance{delta_ns}` messages; QEMU manipulates
`timers_state.qemu_icount_bias` to advance virtual time exactly. This requires icount
mode, which disables translation block chaining — the primary source of the ~5–8×
slowdown. Use only when firmware uses hardware timers to measure intervals shorter than
one physics quantum (e.g., PWM generation, µs-precision DMA bursts).

#### Practical numbers for FirmwareStudio workloads

A typical PID control loop at 1 kHz executes ~10 000 instructions per iteration,
requiring ~10 MIPS effective throughput. Even with icount's 5–8× penalty, a Cortex-A15
emulated in QEMU delivers ~20–40 MIPS — a 2–4× headroom. For 10 kHz loops the margin
tightens; use `slaved-suspend` instead.

---

## 6. Build Environments and `--enable-plugins`

### What `--enable-plugins` provides

`--enable-plugins` enables QEMU's **TCG plugin system** — a stable API for writing
`.so` plugins that instrument every translated instruction, basic block, or memory
access without modifying QEMU source. Bundled plugins include instruction tracers
(`execlog`), coverage recorders (`drcov`, `bbv`), and hardware profilers (`hwprofile`).

For qenode, plugins are useful for:
- Firmware code coverage during Robot Framework test runs
- PC-breakpoint hooks ("stop when firmware reaches address X") without GDB
- Profiling which peripheral MMIO addresses are hottest

Plugins are **not required for Phases 1–4** (device loading, arm-generic-fdt,
repl2qemu, basic QMP testing). They become relevant in Phase 4 (test automation parity
with Renode's tracing features).

### The macOS conflict (GitLab #516)

Building QEMU with **both** `--enable-modules` and `--enable-plugins` on macOS causes
a GLib `g_module_open` symbol visibility conflict that silently breaks module loading.
`--enable-modules` is essential. `--enable-plugins` is not required until Phase 4.

### Recommended build environments

| Scenario | Environment | Plugins | Rationale |
|---|---|---|---|
| Local device development | **Mac or Linux native** | No | Fast iteration: `make build` rebuilds only changed `.c` files |
| Robot Framework test runs | **Docker** (Linux) | Yes | Full tracing and coverage available |
| CI | **Docker** (Linux) | Yes | Consistent, reproducible |
| Production / FirmwareStudio | **Docker** (Linux) | Yes | Matches CI; plugins needed for firmware coverage |

For Phases 1–3, native macOS build is fine and faster for development. When Phase 4
requires plugins, use `docker/docker-compose.yml` even on Mac rather than fighting
the macOS conflict.

```bash
# Native Mac (Phases 1-3, fast dev loop)
make setup && ./scripts/run.sh ...

# Docker (Phases 4+, full plugins, or when matching CI exactly)
docker compose -f docker/docker-compose.yml run cyber-node qemu-system-arm ...
```

`scripts/setup-qemu.sh` automatically detects macOS and omits `--enable-plugins`.

---

## 7. External Time Master: Design and Timing Analysis

### System Context

qenode is the QEMU layer of FirmwareStudio, a digital twin platform where a physics
engine (MuJoCo) simulates the physical world and acts as the **master clock** for all
cyber nodes. Multiple QEMU instances run firmware for different microcontrollers in the
same simulated world. All must advance in lockstep with the physics timestep.

```
┌─────────────────────────────────────────────────────────────────┐
│  FirmwareStudio World                                           │
│                                                                 │
│  ┌──────────────┐   Zenoh   ┌──────────────┐                   │
│  │  MuJoCo      │ ────────► │ TimeAuthority│                   │
│  │  (physics)   │           │  (Python)    │                   │
│  │              │ ◄──────── │              │                   │
│  └──────────────┘  sensors  └──────┬───────┘                   │
│                               actuators                         │
│                                    │ Zenoh GET sim/clock/advance/N
│                                    ▼                            │
│                         ┌──────────────────┐                   │
│                         │  NodeAgent       │                   │
│                         │  (Python)        │                   │
│                         └────────┬─────────┘                   │
│                                  │ Unix socket                  │
│                                  ▼                              │
│                         ┌──────────────────┐                   │
│                         │  QEMU            │                   │
│                         │  + libqemu patch │                   │
│                         │  + qenode hw/    │                   │
│                         └──────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
```

### Wire Protocol

The NodeAgent ↔ QEMU protocol (defined in `patches/apply_libqemu.py` and mirrored in
`tools/node_agent/qemu_clock.py`):

```
Host → QEMU:  ClockAdvance  { uint64 delta_ns;        uint64 mujoco_time_ns; }
QEMU → Host:  ClockReady    { uint64 current_vtime_ns; uint32 n_frames;       }
```

`n_frames` is reserved for Phase 7 (Ethernet frame injection between nodes). Currently
always zero.

### Time Quantum and Causal Consistency

MuJoCo runs at a fixed timestep `dt` (typically 1–10 ms). The TimeAuthority calls
`step(quantum_ns = int(dt * 1e9))` once per `mj_step()`. All QEMU nodes must complete
their quantum before the next physics step begins. This guarantees:

- Sensor values read by firmware are from the same physics tick
- Actuator outputs written by firmware are applied to the next physics tick
- No firmware instance can "see the future" of the physics simulation

### Clock Mode Selection

Choose based on what the firmware measures:

```
Does firmware use hardware timers to measure
intervals SHORTER than one physics quantum (dt)?
         │
         ├── No  → slaved-suspend mode
         │         Full TCG speed. ±dt jitter within step is invisible
         │         to the firmware's control loop.
         │
         └── Yes → slaved-icount mode
                   Exact virtual time. ~5-8x slower. Required for PWM,
                   µs-precision DMA, or tick-counting peripherals.
```

For FirmwareStudio's current workloads (PID at 1–10 kHz, simple sensor polling),
`slaved-suspend` is always sufficient.

### Implications for qenode Peripheral Design

Peripherals that model timers or counters (PWM, SysTick, DWT) must be aware of the
active clock mode and documented accordingly. In `slaved-suspend` mode, a peripheral's
internal tick count only advances when QEMU is running — this is correct behavior and
matches real hardware (the timer only ticks when the MCU is powered).

---

## 8. Prior Art: qbox and MINRES

Two projects address the same problem of coupling QEMU to an external scheduler. Both
were studied when designing qenode's timing architecture.

### Qualcomm qbox (github.com/quic/qbox)

qbox integrates QEMU as a SystemC TLM-2.0 module using two libraries:

- **libqemu-cxx**: C++ wrapper exposing QEMU CPU, interrupt, timer, and PCI devices as
  C++ objects with TLM-2.0 interfaces.
- **libgssync**: Synchronization policy library implementing cooperative suspend/resume
  between QEMU's TCG execution loop and SystemC's event-driven scheduler.

**Key insight adopted by qenode**: `libgssync` does **not** use icount mode. QEMU runs
at full TCG speed between synchronization points. The scheduler suspends QEMU at quantum
boundaries via `vm_stop()` / `vm_start()`, does its work, then resumes. This is the
basis for qenode's `slaved-suspend` mode.

**What qenode does not adopt**: The full SystemC/TLM-2.0 embedding. Our Zenoh message
bus provides the equivalent inter-component communication without requiring SystemC as a
simulation kernel. Zenoh is simpler, language-agnostic, works across containers and
machines, and is already part of FirmwareStudio's infrastructure.

### MINRES libqemu / libqemu-cxx

MINRES describes integrating QEMU as a library within a SystemC virtual platform,
treating QEMU as one component among many rather than as the sole simulator. The
architecture requires significant custom patching per QEMU release.

**Key insight**: The maintainability concern is real and applies to qenode too. Every
QEMU release can break the `libqemu` patch and the `arm-generic-fdt` series. qenode
manages this by:

1. Keeping patches minimal and focused (libqemu: ~150 LOC; arm-generic-fdt: upstream
   series that will eventually merge).
2. Pinning to a specific QEMU ref in the Dockerfile and setup script.
3. Using Python-based patch application (`apply_libqemu.py`) rather than fragile git
   format-patches, making rebasing explicit and auditable.

**What qenode does not adopt**: SystemC as the simulation kernel. Same reasoning as
qbox: Zenoh is sufficient and simpler.

### Summary

| Concern | qbox approach | MINRES approach | qenode approach |
|---|---|---|---|
| Time sync | libgssync suspend/resume | SystemC scheduler | QMP stop/cont (suspend mode) or icount (precise mode) |
| IPC | SystemC TLM-2.0 | SystemC TLM-2.0 | Zenoh + Unix sockets |
| QEMU patching | Heavy (libqemu-cxx) | Heavy (libqemu) | Minimal (libqemu ~150 LOC + arm-generic-fdt) |
| Cross-container | No | No | Yes (Zenoh router) |
| Language | C++ | C++ | Python + C |

---

## 9. SystemC Peripheral Extensions

### Can peripherals be written in SystemC?

Yes. Three integration paths exist, with different complexity/capability tradeoffs:

### Path A — Chardev/vhost-user socket adapter (available now)

Write a SystemC TLM-2.0 module and add a thin C++ adapter that translates TLM
transactions to qenode's Unix socket protocol (the same protocol used by Python daemons).
QEMU maps a `MemoryRegion` to a chardev socket; MMIO reads/writes arrive as byte messages
which the adapter forwards to the SystemC module.

```
Firmware MMIO
    → QEMU chardev socket
        → C++ adapter (thin shim)
            → SystemC TLM-2.0 target socket
                → SystemC peripheral model
```

No QEMU patches needed beyond what is already in qenode. Works in Phases 2+. Best for
**individual peripherals** (sensors, custom IP cores) where the guest software already
exists and only the peripheral model is being replaced.

Limitation: performance depends on IPC round-trip (~1–5 µs per transaction). Acceptable
for peripherals accessed at <1 MHz. Not suitable for DMA engines or fast timers.

### Path B — Remote Port (Phase 5, planned)

QEMU's Remote Port protocol (AMD/Xilinx, used in their QEMU-based virtual platforms)
exposes a QEMU `MemoryRegion` as a TLM-2.0 socket over a Unix socket. A SystemC module
connects to this socket as a standard TLM-2.0 initiator/target.

```
Firmware MMIO
    → QEMU MemoryRegion → Remote Port QOM device
        → Unix socket (Remote Port protocol)
            → SystemC TLM-2.0 target socket
                → SystemC subsystem (Verilated IP, custom hardware model)
```

Remote Port handles time domain synchronization explicitly — the SystemC model is stepped
in sync with QEMU's virtual clock. This is the right path for **co-simulating entire
hardware subsystems** (FPGA fabric, custom processor, multi-component SoC).

This is qenode Phase 5. Depends on `libsystemctlm-soc` (AMD/Xilinx).

### Path C — qbox-style TLM embedding (future consideration)

Qualcomm's qbox wraps QEMU itself as a SystemC component with TLM-2.0 initiator sockets
for each MMIO region. Any SystemC TLM-2.0 peripheral can be connected directly. This
gives the tightest integration and best performance (no extra socket hop) but requires
adopting qbox's `libqemu-cxx` infrastructure.

qenode does not currently use this path. If FirmwareStudio's co-simulation requirements
grow to include many concurrent SystemC peripherals, revisit qbox as the integration
layer for Phase 5+.

### Decision Guide

```
Need to write a peripheral in SystemC?
    │
    ├── Individual device, <1 MHz access rate
    │   → Path A (chardev socket adapter)  ← available now
    │
    ├── Full subsystem co-simulation, Verilator model, FPGA fabric
    │   → Path B (Remote Port)  ← Phase 5
    │
    └── Many SystemC peripherals, tight TLM coupling
        → Path C (qbox)  ← future
```
