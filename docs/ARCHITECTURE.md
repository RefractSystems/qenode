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
| KVM support | Yes (x86, ARM A-profile only) — **not used by qenode** | Yes (x86 only, recent) |

QEMU's pure-C peripheral path is significantly lower latency than Renode's C→C# boundary.
The trade-off: Renode's C# layer enables dynamic peripheral loading without recompilation.

**qenode always uses TCG**, even on ARM hosts. KVM is incompatible with Cortex-M targets
and with icount-based time determinism. See §10 ADR-009.

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

**No Python in the simulation loop.** All peripherals must be authored as native C/Rust
QOM modules and compiled as `.so` files via the Meson module build. Python daemons,
vhost-user backends, and chardev socket proxies are explicitly excluded from the execution
path. The only Python permitted is offline tooling (repl2qemu, pytest).

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

| Renode Keyword | QMP / chardev Translation |
|---|---|
| `Start Emulation` | `{"execute": "cont"}` |
| `Reset Emulation` | `{"execute": "system_reset"}` |
| `Pause Emulation` | `{"execute": "stop"}` |
| `PC Should Be Equal  ${addr}` | `query-cpus-fast` → assert `pc` |
| `Wait For Line On UART  ${pattern}` | chardev socket readline with regex |
| `Execute Command  ${cmd}` | `human-monitor-command` |

UART output capture: QEMU redirects serial to a Unix socket
(`-chardev socket,id=serial0,path=/tmp/qemu_serial,server=on,wait=off`).
The keyword connects to that socket and polls for pattern matches.

**Testing framework**: pytest + `qemu.qmp` is the primary recommendation. It gives
programmatic timeout control (can poll `query-cpus-fast` virtual time rather than
wall-clock), better debuggability, and is what upstream QEMU uses for its own tests.
`qemu_keywords.robot` is kept as a compatibility layer for existing Renode `.robot`
suites, but note that keyword timeouts operate in wall-clock time — this is acceptable
in standalone mode where QEMU runs at ~real-time, but becomes incorrect in `slaved-icount`
mode. Virtual-time-aware timeouts are deferred to Phase 7.

Multi-node (Phase 6): the native Zenoh QOM plugin (`hw/zenoh/zenoh-netdev.c`) acts as
a custom `-netdev` backend, publishing/subscribing Ethernet frames over Zenoh topics with
embedded virtual timestamps. Incoming frames are buffered and delivered to the guest NIC
only when virtual time reaches the stamped arrival time — deterministic by construction,
no UDP multicast or Python coordinator needed.

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

### Phase 3: Peripheral Translation (C# → QOM)
All peripherals — whether performance-critical or low-speed — are implemented as native
C/Rust QOM modules compiled via the Meson module system. Python daemons and chardev socket
proxies are not used. See §10 (ADR-003) for the rationale.

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

- **Pure C/Rust QOM devices**: No C→C# boundary overhead. MMIO latency is significantly
  lower than equivalent Renode peripherals. This is the only permitted implementation path.
- **Python daemons via Unix socket**: NOT used. Each MMIO access crosses a process boundary
  (~1–5 µs round-trip). At 400 kHz I2C bus speed this consumes 400–2000 ms of wall time
  per second of simulated I2C traffic — a catastrophic penalty even for "low-speed"
  peripherals. See ADR-003 for the full reasoning.
- **Profiling**: Use Callgrind + QEMU's TCG Continuous Benchmarking to isolate per-device
  MMIO costs.

### External Clock Performance (Three Modes)

When QEMU is slaved to an external time master (e.g., MuJoCo), there are three operating
modes with very different performance profiles:

| Mode | QEMU flags | Throughput | Use when |
|---|---|---|---|
| `standalone` | (none) | **100%** — full TCG speed | Development, CI without physics |
| `slaved-suspend` | (none — native plugin handles it) | **~95%** — only TB-boundary pause | **Recommended default** for FirmwareStudio |
| `slaved-icount` | `-icount shift=0,align=off,sleep=off` | **~15–20%** — icount disables TB chaining | Only if firmware measures sub-quantum intervals |

#### slaved-suspend (recommended)

At each physics step boundary the native Zenoh plugin (`hw/zenoh/zenoh-clock.c`) blocks
QEMU's TCG loop at a translation-block boundary, waiting for a Zenoh `get` reply from the
TimeAuthority. QEMU runs at **full TCG speed** within each quantum. The only overhead is
the ~10–50 µs Zenoh round-trip at boundaries. No Python process, no QMP socket, no OS
thread scheduling jitter.

This is the pattern used by Qualcomm's **qbox** project via its `libgssync` library
(see Section 8). It gives essentially free-run performance for control loops at 1–10 kHz.

#### slaved-icount (when required)

The native Zenoh plugin (`hw/zenoh/zenoh-clock.c`) also handles slaved-icount: after
blocking on the Zenoh reply it sets `timers_state.qemu_icount_bias += delta_ns` to advance
virtual time exactly the requested amount. QEMU must be started with
`-icount shift=0,align=off,sleep=off`, which disables translation block chaining — the
primary source of the ~5–8× slowdown. Use only when firmware uses hardware timers to
measure intervals shorter than one physics quantum (PWM generation, µs-precision DMA).

#### Practical numbers for FirmwareStudio workloads

A typical PID control loop at 1 kHz executes ~10 000 instructions per iteration,
requiring ~10 MIPS effective throughput. Even with icount's 5–8× penalty, a Cortex-A15
emulated in QEMU delivers ~20–40 MIPS — a 2–4× headroom. For 10 kHz loops the margin
tightens; use `slaved-suspend` instead.

### ARM-on-ARM Hosts (Apple Silicon, AWS Graviton)

A reasonable assumption when deploying on ARM hosts is that QEMU could use hardware
virtualization (KVM on Linux, `hvf` on macOS) to run ARM guest code at near-native
speed, eliminating the TCG overhead entirely. **qenode does not use hardware
virtualization, and cannot.** TCG is mandatory. See ADR-009 for full rationale.

The consequence is a fixed ~10–20% of native throughput from TCG translation overhead,
even on an ARM host. This sounds severe but is irrelevant in practice:

| | Embedded target (STM32F4) | QEMU TCG on M2 Pro | QEMU TCG + icount on M2 Pro |
|---|---|---|---|
| Effective MIPS | ~80–160 MIPS | ~300–600 MIPS | ~20–40 MIPS |
| Headroom vs target | — | **3–7× faster** | **1.5–4× faster** |

Even in the worst case (`slaved-icount` mode on modest hardware), TCG throughput exceeds
the target's real silicon by a comfortable margin. The physics simulation clock — not
QEMU's instruction rate — is the binding constraint for simulation speed.

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
│                                    │ (no Python middleman)      │
│                                    ▼                            │
│                         ┌──────────────────┐                   │
│                         │  QEMU            │                   │
│                         │  + hw/zenoh/     │ ← native C plugin │
│                         │    zenoh-clock.c │   blocks TCG loop  │
│                         │    zenoh-netdev.c│   on Zenoh reply   │
│                         │  + qenode hw/    │                   │
│                         └──────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
```

### Wire Protocol

The TimeAuthority ↔ QEMU protocol is carried directly over Zenoh (no Python middleman).
The native plugin (`hw/zenoh/zenoh-clock.c`) acts as a Zenoh queryable on
`sim/clock/advance/{node_id}`:

```
TimeAuthority → QEMU (Zenoh GET payload):   { uint64 delta_ns; uint64 mujoco_time_ns; }
QEMU → TimeAuthority (Zenoh reply payload): { uint64 current_vtime_ns; uint32 n_frames; }
```

`n_frames` is reserved for Phase 7 (Ethernet frame injection between nodes, delivered via
`sim/eth/frame/ta/{node_id}` Zenoh topics). Currently always zero.

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

### Implementation Constraints for hw/zenoh/

#### BQL (Big QEMU Lock) — zenoh-clock.c

QEMU uses a global lock (BQL, historically `qemu_global_mutex`) to serialize access to
its internal state. The TCG vCPU thread holds the BQL while executing translated guest
code. If the zenoh-clock plugin blocks on a Zenoh network reply *while holding the BQL*,
the main event loop thread (which handles QMP, GDB stub, chardev I/O) cannot acquire the
BQL, causing a total process deadlock. QEMU becomes completely unresponsive — QMP socket
freezes, GDB stub freezes, test harness hangs indefinitely.

**Correct blocking pattern** (must be followed without exception):

```c
/* At the quantum boundary, after cpu_exit() has fired and TCG has exited the
   current TB cleanly: */
bql_unlock();                          /* release before any blocking call */
zenoh_reply = zenoh_get(queryable);    /* block here waiting for TimeAuthority */
bql_lock();                            /* re-acquire before touching QEMU state */
/* Now safe to update timers_state, call cpu_icount_advance(), etc. */
```

`cpu_exit(cpu)` must be called *before* this sequence to request a TB boundary exit; the
actual blocking happens in the outer vCPU dispatch loop, not inside a TB. This is
identical to the pattern used by `qemu_mutex_unlock_iothread()` /
`qemu_mutex_lock_iothread()` in QEMU device models.

#### QEMUTimer — zenoh-netdev.c

QEMU has no mechanism to passively watch a virtual-time threshold and fire a callback
spontaneously. Incoming frames cannot be injected by polling; they must be delivered via
the QEMU timer subsystem.

**Correct virtual-time frame delivery**:

```c
/* At plugin init: */
rx_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, netdev_state);
qemu_mutex_init(&rx_queue_lock);

/* In Zenoh subscription callback (runs in Zenoh thread, NOT QEMU main loop): */
qemu_mutex_lock(&rx_queue_lock);
pqueue_insert(rx_queue, frame, delivery_vtime);
timer_mod(rx_timer, pqueue_min_key(rx_queue));  /* arm for earliest delivery */
qemu_mutex_unlock(&rx_queue_lock);

/* In rx_timer_cb (runs in QEMU main loop, BQL held): */
uint64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
qemu_mutex_lock(&rx_queue_lock);
while (pqueue_min_key(rx_queue) <= now) {
    Frame *f = pqueue_pop(rx_queue);
    qemu_send_packet(nc, f->data, f->len);  /* inject to guest NIC */
    frame_free(f);
}
if (!pqueue_empty(rx_queue))
    timer_mod(rx_timer, pqueue_min_key(rx_queue));  /* re-arm for next frame */
qemu_mutex_unlock(&rx_queue_lock);
```

`QEMU_CLOCK_VIRTUAL` advances with icount in `slaved-icount` mode and with wall-clock
time (gated by QEMU's run state) in `slaved-suspend` mode. In both cases the timer fires
at the correct virtual time.

#### Test isolation — Phase 4 vs Phase 7

Phase 4 pytest tests run without MuJoCo. If QEMU is started with `-device zenoh-clock`
loaded, it will block at the first TCG TB boundary waiting for a TimeAuthority reply that
never arrives. The test process hangs with no timeout or error.

The `qemu_process` pytest fixture must explicitly omit `-device zenoh-clock` from the
QEMU command line. Phase 7 integration tests (which require a running TimeAuthority) are
a separate pytest mark (`@pytest.mark.firmware_studio`) and must not run in standard CI.

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
| Time sync | libgssync suspend/resume | SystemC scheduler | Native Zenoh plugin TB-boundary halt (suspend mode) or icount (precise mode) |
| IPC | SystemC TLM-2.0 | SystemC TLM-2.0 | Zenoh + Unix sockets |
| QEMU patching | Heavy (libqemu-cxx) | Heavy (libqemu) | Minimal (libqemu ~150 LOC + arm-generic-fdt) |
| Cross-container | No | No | Yes (Zenoh router) |
| Language | C++ | C++ | C/Rust (Python for offline tooling only — see §10 ADR-003) |

---

## 9. SystemC Peripheral Extensions

### Can peripherals be written in SystemC?

Yes. Three integration paths exist, with different complexity/capability tradeoffs:

### Path A — Custom MMIO-socket bridge device (lower priority, Phase 5)

> **Important**: QEMU does not natively serialize MMIO accesses into a chardev socket.
> Chardev is a byte-stream backend for UARTs and consoles only. Path A requires a custom
> QOM device (`hw/misc/mmio-socket-bridge.c`) that registers a `MemoryRegion`, intercepts
> `MemoryRegionOps` read/write callbacks, formats `{addr, data, width}` as a
> request-response message, and sends it over a Unix socket. This device must be written
> before Path A is usable.

> **No-Python-in-Loop constraint**: the SystemC adapter on the other end of the socket
> must be a C++ process (not Python). Python daemons are explicitly prohibited.

```
Firmware MMIO
    → hw/misc/mmio-socket-bridge.c (custom QOM device, MemoryRegionOps)
        → Unix socket (request/response protocol)
            → C++ adapter
                → SystemC TLM-2.0 target socket
                    → SystemC peripheral model
```

Best for **individual peripherals** (sensors, custom IP cores) where the guest software
already exists and only the peripheral model is being replaced. Path A is lower priority
than Path B for Phase 5 since Path B (Remote Port) provides a cleaner, standardized
protocol that already handles the MMIO serialization problem.

Limitation: ~1–5 µs round-trip per MMIO access. Acceptable for peripherals at <1 MHz.

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
    │   → Path A  ← Phase 5, requires hw/misc/mmio-socket-bridge.c first
    │              (NOT available until that device is written)
    │
    ├── Full subsystem co-simulation, Verilator model, FPGA fabric
    │   → Path B (Remote Port)  ← Phase 5, primary path
    │
    └── Many SystemC peripherals, tight TLM coupling
        → Path C (qbox)  ← future, if co-simulation requirements grow significantly
```

---

## 10. Architecture Decision Records (ADRs)

This section documents major decisions explicitly, including alternatives that were
considered and rejected. The goal is to prevent the same debates from being re-opened by
new engineers or AI agents in future sessions.

---

### ADR-001: External time sync via native Zenoh QOM plugin, not Python QMP

**Decision**: The FirmwareStudio clock synchronization is implemented as a native C QOM
module (`hw/zenoh/zenoh-clock.c`) that links `zenoh-c` directly and installs cooperative
hooks into QEMU's TCG execution loop.

**Context**: QEMU must halt exactly at quantum boundaries when driven by an external
physics engine (MuJoCo). Three approaches were evaluated:

**Option A — External Python node_agent issuing QMP `stop`/`cont`** *(rejected)*

The initial prototype (`tools/node_agent/`) worked this way. At each physics step the
Python process connected to QEMU's QMP socket and sent `{"execute": "stop"}` followed by
`{"execute": "cont"}` after updating sensor state.

*Why this fails*: QMP commands are dispatched asynchronously through QEMU's main event
loop, which runs in a different OS thread from the TCG execution thread. When Python sends
`stop`, QEMU queues the command but the TCG thread continues executing until the main loop
processes the event — which may be tens to hundreds of milliseconds of virtual time later,
depending on host scheduling. The "stop" boundary is indeterminate from the perspective of
virtual time. This breaks the causal consistency guarantee: firmware may execute past a
physics tick boundary and "see the future" of the simulation.

*Specifically*: This is not the same as qbox's libgssync, which calls `vm_stop()` from
*inside* QEMU's address space, from a thread synchronized with the TCG loop. External QMP
is a fundamentally different (and insufficient) mechanism.

**Option B — libqemu Unix socket patch + Python node_agent** *(retained for slaved-icount only)*

The `patches/apply_libqemu.py` approach adds a Unix socket listener inside QEMU that reads
binary `ClockAdvance` structs and sets `timers_state.qemu_icount_bias`. This gives exact
nanosecond-precision clock advancement, but requires icount mode, which disables TCG
translation block chaining — the primary JIT optimization — resulting in ~5–8× throughput
penalty. A Python process still mediates between Zenoh and this socket.

*Retained as a stepping stone*: `apply_libqemu.py` is used during Phase 1 build setup to
verify the icount path works before `hw/zenoh/` is implemented. In Phase 7 it is
superseded by the native plugin, which handles `qemu_icount_bias` manipulation directly
without a socket or Python process.

**Option C — Native Zenoh QOM plugin with TCG cooperative hooks** *(chosen)*

`hw/zenoh/zenoh-clock.c` is a `SysBusDevice` that links `zenoh-c` and:
1. Installs a callback at TB (translation block) boundaries in the TCG loop
2. At each quantum boundary, blocks the QEMU thread on a Zenoh queryable reply from TimeAuthority
3. Resumes the TCG loop when the reply arrives
4. Optionally sets `qemu_icount_bias` if slaved-icount mode is active

This is identical in principle to qbox's libgssync: the halt happens from inside QEMU at
a well-defined TCG yielding point. The module compiles as `hw-qenode-zenoh.so` via the
existing Meson module system — no special patching beyond the hook registration.

**Performance consequences**:
- `slaved-suspend`: TB-boundary halt → ±1 translation block jitter (typically <1000
  instructions). For 1 kHz PID loops this jitter is ~10 µs of virtual time — invisible.
- `slaved-icount`: exact nanosecond advance, but ~5–8× throughput penalty. Reserved for
  firmware that measures intervals shorter than one physics quantum (PWM, DWT).

---

### ADR-002: UDP multicast netdev replaced by Zenoh virtual-timestamp netdev

**Decision**: Multi-node networking for Phase 6 uses a custom `-netdev` backend
(`hw/zenoh/zenoh-netdev.c`) that routes frames through Zenoh with embedded virtual
timestamps, not QEMU's built-in `-netdev socket,mcast=...`.

**Context**: Renode provides a `WirelessMedium` with deterministic distance-based packet
loss. Reproducing this requires deterministic packet delivery in virtual time.

**Option A — `-netdev socket,mcast=230.0.0.1:1234` + Python coordinator** *(rejected)*

QEMU's built-in multicast socket support lets multiple instances share a virtual network.
A Python coordinator script intercepts multicast traffic, applies attenuation, and
rebroadcasts.

*Why this fails*: UDP datagram delivery to a QEMU process is controlled by the host Linux
kernel network stack. Even if QEMU runs in icount mode, when a UDP packet arrives at
QEMU's file descriptor depends on the kernel's socket buffer scheduling — not on QEMU's
virtual clock. Two identical runs will see packets at different virtual timestamps. The
Python coordinator adds another layer of OS-scheduled jitter. icount mode makes QEMU's
internal state reproducible but cannot control external socket delivery timing.

*Additionally*: violates the No-Python-in-Loop mandate — see ADR-003.

**Option B — Zenoh virtual-timestamp netdev** *(chosen)*

`hw/zenoh/zenoh-netdev.c` is a custom QEMU `NetClientInfo` backend:
- TX: firmware writes a frame to the virtual NIC → netdev publishes it to
  `sim/eth/frame/{node_id}/tx` with the current virtual time as a header field
- A lightweight C/Rust coordinator subscribes all TX topics, applies attenuation, and
  republishes to `sim/eth/frame/{dst_id}/rx` with `delivery_vtime = send_vtime + latency`
- RX: netdev subscribes its RX topic, buffers incoming frames keyed by `delivery_vtime`,
  and injects them into the guest NIC only when the QEMU instance's virtual time reaches
  `delivery_vtime`

Determinism comes from virtual-timestamp ordering, not from OS socket delivery timing.
Two identical runs produce identical packet delivery sequences because the coordinator's
attenuation calculation is deterministic and delivery is gated on virtual time.

---

### ADR-003: No Python in the simulation execution loop

**Decision**: Python is explicitly prohibited from any path that runs during QEMU's
execution. Permitted Python: `tools/repl2qemu/` (offline), `tools/testing/` (test harness
that drives QEMU externally, not inline with its execution).

**Context**: Several early design ideas used Python for runtime roles:
1. `tools/node_agent/` — Zenoh ↔ QMP bridge for clock sync
2. vhost-user Python daemons — peripheral models for I2C sensors, SPI config registers
3. chardev socket Python handlers — simple byte-stream peripherals

**Why Python in the loop was rejected**:

*IPC latency*: Any Python process reached via Unix socket during QEMU execution incurs
~1–5 µs of round-trip latency per call due to kernel context switches + Python GIL
contention. At 1 MHz peripheral access rates this adds 1–5 seconds of wall time per
second of simulated time — a catastrophic slowdown. Even "low-speed" I2C peripherals
(400 kHz bus) hit this ceiling in interrupt-driven firmware.

*Determinism jitter for clock sync*: As documented in ADR-001, Python QMP stop/cont
introduces non-deterministic virtual-time boundaries. No amount of effort makes external
Python capable of halting QEMU at a precisely defined virtual-time point.

*Process scheduling unpredictability*: Python's GIL and the host OS scheduler introduce
latency spikes (1–100 ms) that, under heavy CI load or Docker resource contention, cause
random test failures unrelated to firmware correctness.

**What this means in practice**:
- All peripheral models: native C/Rust `SysBusDevice` compiled as `.so`
- Clock sync: `hw/zenoh/zenoh-clock.c` (C, compiled into QEMU)
- Multi-node networking: `hw/zenoh/zenoh-netdev.c` (C, compiled into QEMU)
- Test harness: pytest + `qemu.qmp` runs *outside* QEMU (acceptable — it drives QEMU,
  it is not inline with QEMU's execution)
- Robot Framework keywords: wrapper over the pytest/QMP layer, also runs outside QEMU

---

### ADR-004: pytest as primary test framework; Robot Framework as compatibility layer

**Decision**: New tests are written in pytest. `qemu_keywords.robot` exists as a thin
wrapper to run existing Renode `.robot` suites against QEMU with minimal changes.

**Context**: Renode's test ecosystem uses Robot Framework. Maintaining keyword parity was
an original goal.

**Why Robot Framework was downgraded**:

1. *Timeout semantics*: Robot Framework's `Wait For Line On UART timeout=10` measures
   10 seconds of wall-clock time. In `slaved-icount` mode QEMU runs at ~15% speed, so 10
   wall-clock seconds cover only ~1.5 virtual seconds. Tests fail randomly under CI load
   with no firmware bug. pytest lets you write `wait_for_uart(pattern, vtime_limit_ns=10e9)`
   by tracking the accumulated `delta_ns` reported in Zenoh clock replies.

2. *Debuggability*: Robot Framework's tabular DSL makes it hard to add conditional logic,
   loop over test cases, or debug with a Python debugger. pytest provides full IDE support
   and `--pdb` drop-in on failure.

3. *QEMU upstream alignment*: QEMU's own test suite uses pytest + `qemu.qmp`. Our test
   helper code is compatible with upstream QEMU test infrastructure.

**Why Robot Framework is kept at all**: Existing Renode test suites are written in `.robot`.
Throwing away keyword compatibility breaks the migration path for projects moving from
Renode to qenode. `qemu_keywords.robot` maps Renode keywords to QMP calls so those suites
run without keyword-level rewrites. The timeout issue is documented and acceptable for
`standalone` mode (QEMU at ~real-time) which covers most CI scenarios.

---

### ADR-005: SystemC Path A requires a custom MMIO-socket bridge device

**Decision**: ARCHITECTURE.md §9 Path A (individual SystemC peripheral via socket) is
not available without first writing `hw/misc/mmio-socket-bridge.c`.

**Context**: Early documentation stated "QEMU maps a MemoryRegion to a chardev socket;
MMIO reads/writes arrive as byte messages." This was incorrect.

**The actual QEMU model**: A chardev is a byte-stream abstraction backed by a PTY, socket,
file, or pipe. It is designed for UARTs and consoles: sequential byte streams with no
notion of addresses or access widths. There is no QEMU mechanism that serializes
`MemoryRegionOps.read(addr, size)` / `write(addr, data, size)` callbacks into a chardev
stream. The two subsystems (MMIO and chardev) are entirely separate.

**What Path A actually requires**: A custom `SysBusDevice` (`hw/misc/mmio-socket-bridge.c`)
that:
1. Registers a `MemoryRegion` for a configurable address range
2. Implements `MemoryRegionOps.read` and `.write` by serializing
   `{type, addr, data, size}` into a binary framed message over a Unix socket
3. For reads, blocks waiting for a reply `{data}` from the external adapter
4. On the other end: a C++ SystemC adapter that deserializes these messages into
   `b_transport` calls

This device is implementable within qenode's existing module framework (it's just another
`.so`), but it must be written before any Path A SystemC integration is possible.

Path B (Remote Port) is preferred for Phase 5 because the MMIO serialization problem is
already solved by the Remote Port protocol, which was designed exactly for this use case.

---

### ADR-007: BQL must be released before any blocking call in the vCPU thread

**Decision**: `zenoh-clock.c` calls `bql_unlock()` immediately before blocking on a
Zenoh reply and `bql_lock()` immediately after, without exception.

**Context**: QEMU's Big QEMU Lock (BQL) serializes all access to QEMU's internal device
and CPU state. The TCG vCPU thread holds the BQL continuously while executing translated
guest code. All of QEMU's I/O servicing, QMP socket handling, and GDB stub processing
happen in the main event loop thread, which must also acquire the BQL to do anything.

**Why naive blocking deadlocks**:

If `zenoh_get()` is called while the vCPU thread holds the BQL, the call blocks. The
main event loop thread then spins trying to acquire the BQL and cannot proceed. QMP
becomes unresponsive. The GDB stub freezes. The pytest fixture's `qmp_bridge.execute()`
call hangs with no timeout or error. The only observable symptom is that the test process
hangs indefinitely — there is no crash, no assert, no log message.

**Why this is easy to get wrong**: The BQL is implicit. No function signature tells you
"this function must be called with the BQL held." You must know QEMU's execution model to
understand when you hold it and when you don't. Any blocking network, socket, or
semaphore call inside the TCG execution path is potentially a deadlock.

**The correct pattern** (see §7 implementation constraints for full code):

```c
cpu_exit(cpu);       /* request clean TB boundary exit — do NOT block mid-TB */
/* ... outer vCPU loop fires our hook between TB dispatches ... */
bql_unlock();
zenoh_get(...);      /* safe to block here */
bql_lock();
/* proceed to update QEMU state */
```

**Precedent in QEMU**: `qemu_mutex_unlock_iothread()` / `qemu_mutex_lock_iothread()`
(aliases for bql_unlock/bql_lock) appear hundreds of times in QEMU's codebase wherever
the vCPU thread must block or yield for any reason.

---

### ADR-008: Virtual-time frame delivery requires QEMUTimer, not passive polling

**Decision**: `zenoh-netdev.c` uses `timer_new_ns(QEMU_CLOCK_VIRTUAL, ...)` with a
priority queue to deliver Ethernet frames at their designated virtual arrival time.

**Context**: The Zenoh coordinator attaches a `delivery_vtime` to each incoming Ethernet
frame. The netdev must not inject the frame into the guest NIC until QEMU's virtual clock
reaches `delivery_vtime`. This is how deterministic wireless medium simulation works —
packet "travel time" is expressed as a virtual-time delta, not a wall-clock delay.

**Why passive polling does not work**:

QEMU does not run a background thread that monitors arbitrary conditions. There is no
"watch this value and fire a callback when it crosses a threshold" API. The virtual clock
does not self-advance (in slaved modes it advances only when the physics engine grants
time). A loop like `while (qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) < delivery_vtime) {}`
inside any QEMU thread would either spin-waste CPU or deadlock (if called from the vCPU
thread while holding the BQL).

**Why QEMUTimer is the correct mechanism**:

QEMU's timer subsystem (`QEMUTimer`) is specifically designed for this: register a
callback at a virtual time; QEMU calls it when the virtual clock reaches that time. In
`slaved-icount` mode the virtual clock is the icount clock; in `slaved-suspend` mode it
advances as QEMU resumes between physics steps. In both cases timers fire at the correct
virtual time. `qemu_send_packet()` is only callable from the main QEMU thread (or with
the BQL held), which is exactly where timer callbacks run.

**Thread safety**: The Zenoh subscription callback runs in Zenoh's internal thread pool,
not in the QEMU main loop. The priority queue is therefore shared between two threads and
must be protected by a `QemuMutex`. `timer_mod()` is safe to call from any thread.

---

### ADR-009: TCG is mandatory — KVM/hvf hardware acceleration is not used

**Decision**: qenode always uses QEMU's TCG (software JIT) execution engine, even when
running on an ARM host that could in principle use KVM or Apple Hypervisor.framework for
near-native ARM execution.

**Context**: When FirmwareStudio is deployed on an ARM host (Apple M-series, AWS
Graviton), a reasonable optimization would be to enable KVM (`-accel kvm`) or `hvf`
(`-accel hvf`) so that guest ARM instructions execute directly on the host CPU without
TCG translation overhead. This would give roughly 100% of native speed instead of the
~10–20% delivered by TCG. The question was examined and rejected on two independent
grounds.

**Reason 1 — CPU profile incompatibility for Cortex-M targets**

KVM and hvf execute guest instructions on the bare host CPU. The host CPU must support
the same CPU *profile* as the guest. ARM host CPUs (Apple M-series, Graviton) are
**A-profile** (Application: ARMv8-A / ARMv9-A). The vast majority of embedded firmware
targets in FirmwareStudio are **M-profile** (Microcontroller: ARMv7-M / ARMv8-M,
Cortex-M3/M4/M7/M33).

A-profile and M-profile are not just instruction set variants — they have fundamentally
different exception models, memory protection architectures, and execution states
(EL0/EL1/EL2 vs Thread/Handler mode with Thumb-only execution). A host A-profile CPU
cannot natively execute M-profile firmware. There is no KVM path for Cortex-M simulation;
TCG is the only option.

Even for Cortex-A targets (e.g., a Raspberry Pi firmware): the specific version and
extensions must match. Simulating a Cortex-A15 (ARMv7-A) on an M2 (ARMv9-A) with KVM
would require the host to support ARMv7 guest state, which macOS hvf does not expose.

**Reason 2 — KVM is incompatible with deterministic time control**

Even for targets where KVM is technically possible, it destroys the time determinism that
FirmwareStudio requires:

- KVM executes instructions at full hardware speed with no instruction counting. There
  is no virtual instruction counter, no `icount_decr`, no TB boundary event. `-icount`
  mode is explicitly TCG-only; it cannot be combined with `-accel kvm`.
- The `zenoh-clock.c` plugin's cooperative halt mechanism (ADR-001) relies on TCG's TB
  boundary hooks (`cpu_exit()`, `icount_decr`). Under KVM, these hooks do not exist.
  The only way to halt a KVM vCPU thread is via a host OS signal or `ioctl(KVM_RUN)`
  return, both of which are subject to host OS scheduling jitter of 50–1000 µs — far
  beyond the sub-quantum precision required for causal consistency with MuJoCo.
- Without exact quantum boundaries, firmware can execute past a physics tick, read stale
  sensor values, and produce non-deterministic actuator outputs. The digital twin is
  broken.

**Performance consequence**

TCG on a modern ARM host (Apple M2 Pro, AWS Graviton 3) delivers ~300–600 MIPS for
typical Cortex-M firmware. The target silicon (STM32F4: ~80–160 MIPS) is 3–7× slower
than TCG throughput. Even in `slaved-icount` mode (5–10× TCG penalty), effective
throughput remains ~20–40 MIPS — still 1.5–4× faster than the real chip. The TCG
penalty is absorbed entirely by host hardware headroom.

**Verdict**: Do not add KVM/hvf support. It is categorically incompatible with Cortex-M
targets and with FirmwareStudio's time synchronization model. This decision is not a
cost/benefit trade-off — it is a hard technical constraint.

---

### ADR-006: apply_libqemu.py is a Phase 1-6 stepping stone, superseded in Phase 7

**Decision**: `patches/apply_libqemu.py` is kept through Phase 6 for `slaved-icount` mode
testing. In Phase 7 it is superseded by `hw/zenoh/zenoh-clock.c` and removed.

**Context**: `apply_libqemu.py` AST-injects code into QEMU's `cpus.c` and `icount.c` to
expose a Unix socket that accepts binary `ClockAdvance` structs and directly manipulates
`timers_state.qemu_icount_bias`. The Python node_agent bridged Zenoh to this socket.

**Why it was written in the first place**: It proved the icount-based clock control
mechanism before `hw/zenoh/` was designed. It replaced FirmwareStudio's fragile `.patch`
file with an AST-aware injection that survives minor QEMU refactors.

**Why it is superseded**: `hw/zenoh/zenoh-clock.c` operates inside QEMU's address space
and can call `timers_state.qemu_icount_bias` and `cpu_exit()` directly without any socket.
The external socket + Python mediator was an artifact of implementing this outside QEMU.
Once the native module exists, the patch and its socket are pure overhead.

**The transition**: Phase 1 setup still runs `apply_libqemu.py` to validate icount-mode
clock advancement during early development (before Phase 7 hw/zenoh/ is written). Phase 7
task 7.1 writes the native plugin; task 7.3 removes `tools/node_agent/` and the clocksock
patch injection from `scripts/setup-qemu.sh`.
