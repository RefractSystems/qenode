# virtmcu Architecture

## 1. What virtmcu Is

virtmcu is a **deterministic multi-node firmware simulation framework** built on QEMU.
It is the QEMU layer of **FirmwareStudio**, a digital twin platform for embedded systems
where a physics engine (MuJoCo) simulates the physical world and acts as the master clock
for all cyber nodes.

The core thesis: firmware for cyber-physical systems cannot be tested in isolation. It
reads sensors, drives actuators, and communicates with peer microcontrollers — all of
which unfold in physical time. Correct simulation of this system requires that every
virtual microcontroller shares the same notion of time, that inter-node communication is
deterministically ordered by virtual time (not wall-clock scheduling), and that the
boundary between firmware registers and physical quantities is explicitly modeled.

virtmcu addresses these requirements at the QEMU layer, using native C/Rust QOM modules
linked directly into the emulator. No Python daemons run in the simulation loop.

### What This Is Not

virtmcu is not a fork of QEMU. It is not a re-implementation of Renode. It started with
the goal of providing Renode's ergonomics (dynamic machine descriptions, hot-pluggable
peripherals, Robot Framework testing) on top of QEMU's performance. That goal remains, but
the more important work is the **deterministic distributed simulation infrastructure**:
cooperative time slaving, virtual-timestamped multi-node communication, and the
sensor/actuator abstraction layer. These capabilities have no direct equivalent in Renode.

---

## 2. System Context

```
┌──────────────────────────────────────────────────────────────────────┐
│  FirmwareStudio World                                                │
│                                                                      │
│  ┌──────────────┐   mj_step()   ┌──────────────────┐               │
│  │  MuJoCo      │ ────────────► │  TimeAuthority   │               │
│  │  (physics)   │               │  (Python)        │               │
│  │              │ ◄──────────── │                  │               │
│  └──────────────┘  sensor data  └────────┬─────────┘               │
│                                          │                           │
│                     Zenoh GET sim/clock/advance/{node_id}           │
│                     (no Python middleman — native C plugin)         │
│                                          │                           │
│              ┌───────────────────────────┼────────────────────┐     │
│              │  QEMU node 0              │  QEMU node 1       │     │
│              │  + hw/zenoh/              │  + hw/zenoh/       │     │
│              │    zenoh-clock.c  ◄───────┘    zenoh-clock.c   │     │
│              │    zenoh-netdev.c ◄────────────zenoh-netdev.c  │     │
│              │    zenoh-chardev.c◄────────────zenoh-chardev.c │     │
│              │  + QOM peripherals        │  + QOM peripherals │     │
│              │    (SAL/AAL boundary)     │    (SAL/AAL boundary)│    │
│              │                           │                    │     │
│              │  firmware (bare-metal C)  │  firmware          │     │
│              └───────────────────────────┴────────────────────┘     │
└──────────────────────────────────────────────────────────────────────┘
```

All inter-node communication — Ethernet frames, UART bytes, clock quanta — flows through
**Zenoh** as the federation bus. There are no UDP sockets, no Python bridges, no shared
memory between nodes. Virtual timestamps embedded in each message enforce causal ordering:
a frame sent at virtual time T by node 0 cannot be read by node 1 until its virtual clock
reaches T plus the modeled propagation delay.

---

## 3. The Five Pillars

### Pillar 1 — Cooperative Time Slaving

QEMU's virtual clock must not free-run in a cyber-physical simulation. If firmware on MCU-A
writes a PWM value at virtual time T=5 ms, the physics engine must model that output at
T=5 ms — not at whatever wall-clock moment the QEMU process happened to execute that
instruction. This requires QEMU to be a **time slave**: it runs at full TCG speed within
each quantum but blocks at every quantum boundary until the external TimeAuthority grants
the next advance.

**Implementation**: `hw/zenoh/zenoh-clock.c` is a native QOM device that:
1. Hooks into the TCG execution loop via the `virtmcu_tcg_quantum_hook` function pointer
   injected into `cpu-exec.c` by `patches/apply_zenoh_hook.py`.
2. At each quantum boundary, calls `cpu_exit()` to request a clean translation-block exit,
   releases the BQL, and blocks on a Zenoh `GET` to `sim/clock/advance/{node_id}`.
3. On reply, re-acquires the BQL and optionally advances `timers_state.qemu_icount_bias`
   for exact nanosecond virtual time in `slaved-icount` mode.

**Two slaved modes**:

| Mode | QEMU flags | Throughput | Use when |
|---|---|---|---|
| `slaved-suspend` | (none) | **~95%** — only TB-boundary pause | **Default.** Control loops ≥ one quantum. |
| `slaved-icount` | `-icount shift=0,align=off,sleep=off` | **~15–20%** | Firmware measures sub-quantum intervals (PWM, µs DMA). |

`standalone` mode (no zenoh-clock device) runs at full TCG speed, used for development
and CI without a physics engine.

**BQL constraint**: The Zenoh `GET` call must always be made with the BQL released.
Blocking while holding the BQL deadlocks the QEMU process — the main event loop (QMP,
GDB stub, chardev I/O) cannot acquire the lock. The correct pattern:

```c
bql_unlock();
zenoh_reply = zenoh_get(queryable);  /* blocks here */
bql_lock();
/* now safe to update timers_state */
```

### Pillar 2 — Deterministic Multi-Node Communication

In a multi-node simulation, every message crossing a node boundary must carry a virtual
timestamp. The receiving node's delivery machinery must not inject the message into the
guest until its virtual clock reaches the stamped time. This is the only way to make
inter-node behavior reproducible across runs, regardless of host scheduling.

**Ethernet** (`hw/zenoh/zenoh-netdev.c`): A custom `-netdev` backend that:
- On TX: reads the current `QEMU_CLOCK_VIRTUAL` value, attaches it as a header, publishes
  the frame to `sim/eth/frame/{src_node}/tx` on Zenoh.
- On RX: a Zenoh subscriber receives frames into a priority queue keyed by delivery
  virtual time. A `QEMUTimer` on `QEMU_CLOCK_VIRTUAL` fires when the earliest queued
  frame's timestamp is reached, injecting it into the guest NIC via `qemu_send_packet`.

**UART** (`hw/zenoh/zenoh-chardev.c`): The same virtual-timestamp model applied to serial
bytes, using QEMU's chardev backend API. Enables multi-node UART communication (e.g.,
firmware on MCU-A sends a command string to MCU-B's UART) with correct virtual ordering.
Also supports human-in-the-loop interactivity — terminal input is delivered at the correct
virtual time rather than injected at whatever wall-clock moment the user typed.

**Wire protocol** (TimeAuthority ↔ QEMU):
```
GET sim/clock/advance/{node_id}
  payload → { uint64 delta_ns; uint64 mujoco_time_ns; }
  reply   ← { uint64 current_vtime_ns; uint32 n_pending_frames; }
```

### Pillar 3 — Sensor/Actuator Abstraction (SAL/AAL)

Firmware speaks binary: register reads return 16-bit ADC counts, register writes set
16-bit duty cycles. Physics speaks continuous: acceleration in m/s², torque in N·m.
Bridging these two worlds is the **Sensor/Actuator Abstraction Layer**.

The SAL/AAL lives at the QOM peripheral boundary:
- **Actuator peripherals** (PWM, DAC, GPIO output): decode firmware register writes into
  physical quantities. A motor PWM peripheral converts duty cycle → voltage → expected
  torque. The result is published over Zenoh to the physics engine.
- **Sensor peripherals** (ADC, IMU, encoder): receive physical quantities from the physics
  engine over Zenoh and encode them into firmware-readable register values, applying
  configurable noise models and transfer functions.

**Two operating modes**:
- *Standalone (RESD)*: Sensor values are replayed from Renode Sensor Data binary files.
  No physics engine required. Deterministic, fast, suitable for CI/CD regression testing.
- *Integrated (MuJoCo)*: Zero-copy `mjData` shared memory provides live physics state.
  Actuator outputs are applied to MuJoCo before the next `mj_step()`.

### Pillar 4 — Dynamic Machines and QOM Plugin Infrastructure

QEMU traditionally requires recompiling the emulator to add a new device or define a new
machine. virtmcu eliminates both constraints.

**Dynamic machines** (`arm-generic-fdt` patch series): A new ARM machine type that
instantiates CPUs, memory, and peripherals entirely from a Device Tree blob at runtime.
`-machine arm-generic-fdt -hw-dtb board.dtb` replaces the hardcoded C machine struct.

**Dynamic QOM plugins**: `hw/` is symlinked into QEMU's source tree and compiled as proper
QEMU modules (`--enable-modules`). The resulting `.so` files are auto-discovered via
QEMU's `module_info` table. `-device my-peripheral` loads the `.so` without `LD_PRELOAD`.
All peripherals are native C or Rust (via FFI) — no Python daemons, no vhost-user proxies.

**Platform description tools** (`tools/repl2qemu/`): Compile legacy Renode `.repl` files
or OpenUSD-aligned `.yaml` board descriptions into Device Tree blobs and QEMU CLI strings.
This is the reverse of Antmicro's `dts2repl`.

### Pillar 5 — Co-Simulation with External Hardware Models

For projects with Verilated C++ hardware models or real FPGA hardware:

**SystemC TLM-2.0 (Phase 5)**: Replace Renode's `IntegrationLibrary` headers with
AMD/Xilinx `libsystemctlm-soc`. Wrap Verilated models as SystemC TLM-2.0 modules and
connect to QEMU via Remote Port Unix sockets. Remote Port handles time domain
synchronization.

**Shared physical media (Phase 9)**: Model CAN buses, SPI buses, and similar shared media
in SystemC. A multi-threaded C++ adapter translates QEMU MMIO to TLM-2.0 calls and
handles asynchronous `IRQ_SET`/`IRQ_CLEAR` messages without blocking the SystemC scheduler.

**EtherBone (FPGA over UDP)**: A custom QOM device intercepts MMIO writes, constructs
EtherBone packets, and sends them over UDP — mirroring Renode's `EtherBoneBridge`.

---

## 4. QEMU Build Details

### Version and Patches

- **Base**: QEMU 11.0.0-rc3 (internally versioned as 10.2.92; git tag `v11.0.0-rc3`)
- **Patches applied in order by `scripts/setup-qemu.sh`**:
  1. `patches/arm-generic-fdt-v3.mbx` — 33-patch series (patchew ID
     `20260402215629.745866-1-ruslichenko.r@gmail.com`), applied via `git am`
  2. `patches/apply_zenoh_hook.py` — AST-injects `virtmcu_tcg_quantum_hook` function
     pointer into `accel/tcg/cpu-exec.c`
  3. `patches/apply_zenoh_netdev.py` — registers the Zenoh netdev backend

- **Required configure flags**:
  ```
  --enable-modules --enable-fdt
  --target-list=arm-softmmu
  ```

### Module Build Integration

`scripts/setup-qemu.sh` creates a symlink:
```
third_party/qemu/hw/virtmcu  →  <repo>/hw/
```
and appends `subdir('virtmcu')` to `third_party/qemu/hw/meson.build`.

`hw/meson.build` registers modules in QEMU's `modules` dict:
```meson
modules += {'hw-virtmcu': hw_virtmcu_modules}
```

With `--enable-modules`, this produces `hw-virtmcu-zenoh.so`, `hw-virtmcu-dummy.so`, etc.,
installed in `QEMU_MODDIR`. `QEMU_MODULE_DIR` is set by `scripts/run.sh`.

### zenoh-c dependency

`hw/zenoh/` links `zenoh-c` directly. In development: `third_party/zenoh-c/` (pre-built,
not in git). In Docker: built from source at `ZENOH_C_REF` (default `1.8.0`).
`hw/zenoh/meson.build` resolves `meson.project_source_root()/../zenoh-c`, which is
correct for both `third_party/qemu` (local) and `/build/qemu` (Docker).

---

## 5. Timing Design and Performance

### Clock Mode Selection

```
Does firmware use hardware timers to measure
intervals SHORTER than one physics quantum (dt)?
         │
         ├── No  → slaved-suspend mode
         │         Full TCG speed. ±dt jitter within step is invisible
         │         to the firmware's control loop.
         │
         └── Yes → slaved-icount mode
                   Exact virtual time. ~5–8× slower. Required for PWM,
                   µs-precision DMA, or tick-counting peripherals.
```

For FirmwareStudio workloads (PID at 1–10 kHz, sensor polling), `slaved-suspend` is
always sufficient. A typical 1 kHz PID loop executes ~10 000 instructions per iteration;
QEMU TCG delivers 300–600 MIPS in standalone mode, ample headroom even with the TB-boundary
pause overhead.

### Performance Table

| Mode | Effective throughput | Limiting factor |
|---|---|---|
| `standalone` | 300–600 MIPS (TCG) / 1–2 GIPS (KVM/hvf, Cortex-A only) | Host CPU |
| `slaved-suspend` | ~95% of standalone | ~10–50 µs Zenoh round-trip per quantum |
| `slaved-icount` | ~20–40 MIPS | TB chaining disabled by `-icount` |

### QEMUTimer for Frame Delivery

QEMU has no mechanism to passively watch a virtual-time threshold. Incoming frames cannot
be injected by polling; they must use the QEMU timer subsystem:

```c
/* Init: */
rx_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, state);

/* In Zenoh subscriber callback (Zenoh thread, NOT QEMU main loop): */
qemu_mutex_lock(&rx_queue_lock);
pqueue_insert(rx_queue, frame, delivery_vtime);
timer_mod(rx_timer, pqueue_min_key(rx_queue));
qemu_mutex_unlock(&rx_queue_lock);

/* In rx_timer_cb (QEMU main loop, BQL held): */
uint64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
while (pqueue_min_key(rx_queue) <= now) {
    Frame *f = pqueue_pop(rx_queue);
    qemu_send_packet(nc, f->data, f->len);
    frame_free(f);
}
if (!pqueue_empty(rx_queue))
    timer_mod(rx_timer, pqueue_min_key(rx_queue));
```

`QEMU_CLOCK_VIRTUAL` advances with icount in `slaved-icount` mode and with QEMU's run
state (gated by `vm_stop`/`vm_start`) in `slaved-suspend` mode.

### ARM-on-ARM Hosts (Apple Silicon, AWS Graviton)

KVM/hvf acceleration is only available in `standalone` mode with Cortex-A targets. It is
prohibited in all slaved modes (cooperative hooks require TCG internals that KVM bypasses)
and for all Cortex-M targets (hypervisors do not support M-profile).

---

## 6. Prior Art

### Qualcomm qbox (github.com/quic/qbox)

qbox integrates QEMU as a SystemC TLM-2.0 module using `libqemu-cxx` (C++ wrapper) and
`libgssync` (synchronization policy). **Key insight**: `libgssync` does not use icount
mode. QEMU runs at full TCG speed within each quantum; the synchronization library
suspends at quantum boundaries via `vm_stop()`/`vm_start()`. This is the conceptual basis
for virtmcu's `slaved-suspend` mode.

**What virtmcu does differently**: Instead of SystemC as the simulation kernel, Zenoh
acts as the inter-component bus. Zenoh is language-agnostic, works across containers and
machines, and is already part of FirmwareStudio's infrastructure. The cooperative suspend
mechanism is equivalent to qbox's but implemented as a native QOM module rather than a
C++ SystemC wrapper.

### MINRES libqemu

MINRES integrates QEMU as a library within a SystemC virtual platform. More invasive than
qbox — requires significant custom patching per QEMU release.

**Key insight for virtmcu**: The maintainability concern is real. Every QEMU release can
break the `arm-generic-fdt` series and the TCG hook patch. virtmcu manages this by keeping
patches minimal, pinning to a specific QEMU ref, and using auditable Python-based AST
injection rather than fragile format-patches.

**What virtmcu does not adopt**: SystemC as the simulation kernel. Zenoh provides the
equivalent of TLM-2.0 transaction semantics across a network without the SystemC dependency.

---

## 7. Build Environments

### `--enable-plugins` and the macOS conflict

`--enable-plugins` enables QEMU's TCG plugin API (instruction tracing, coverage, MMIO
profiling). Required for Phase 4+ test automation features.

Building with both `--enable-modules` and `--enable-plugins` on macOS causes a GLib
`g_module_open` symbol conflict (GitLab #516) that silently breaks module loading.
`--enable-modules` is essential; `--enable-plugins` is not required until Phase 4.

| Scenario | Environment | Plugins |
|---|---|---|
| Phase 1–3 peripheral dev | Native Mac or Linux | No |
| Phase 4+ test automation | Docker (Linux) | Yes |
| CI | Docker (Linux) | Yes |
| FirmwareStudio production | Docker (Linux) | Yes |

`scripts/setup-qemu.sh` automatically detects macOS and omits `--enable-plugins`.

---

## 8. Architectural Decision Records

### ADR-001: Three clock modes (standalone / slaved-suspend / slaved-icount)

**Decision**: Implement three distinct clock modes rather than a single unified approach.
**Rationale**: `slaved-icount` is required for sub-quantum timer precision but costs 5–8×
throughput. Making it the default would unnecessarily penalize the 95% of workloads that
do not need nanosecond accuracy within a quantum. `standalone` mode is essential for
development and CI without a physics engine.

### ADR-002: Zenoh for all inter-node traffic

**Decision**: Use Zenoh as the sole message bus for clock quanta, Ethernet frames, UART
bytes, and sensor data.
**Rationale**: A single bus simplifies the operational model. Zenoh is language-agnostic
(C, Rust, Python clients all available), works across containers and physical machines
without VPN or network configuration, and is already part of FirmwareStudio infrastructure.
UDP multicast (the QEMU default for multi-node) is non-deterministic and does not support
virtual-timestamp delivery.

### ADR-003: No Python in the simulation loop

**Decision**: All devices, clock sync, and networking in the simulation loop must be
native C or Rust QOM modules.
**Rationale**: Each MMIO access that crosses a process boundary (Unix socket to a Python
daemon) adds ~1–5 µs round-trip latency. At 400 kHz I2C bus speed this is 400–2000 ms of
wall time per simulated second — a catastrophic penalty for even "low-speed" peripherals.
The vhost-user protocol has the same problem. Python is only permitted for offline tooling
(repl2qemu, pytest, test harness scripts).

### ADR-004: Virtual-timestamp delivery for all inter-node messages

**Decision**: Every message crossing a QEMU node boundary carries an embedded virtual
timestamp and is held in a priority queue until the receiving node's virtual clock reaches
that timestamp.
**Rationale**: Without virtual-timestamp delivery, the ordering of messages between nodes
is determined by wall-clock scheduling — non-deterministic, host-load-dependent, and
therefore not reproducible. The priority-queue + QEMUTimer pattern is the only correct
implementation given QEMU's timer subsystem semantics.

### ADR-005: SystemC for co-simulation, not for the main simulation kernel

**Decision**: SystemC TLM-2.0 is used for co-simulation with external Verilated models
(Phase 5, 9) but is not the top-level simulation kernel.
**Rationale**: SystemC as a kernel (qbox / MINRES approach) requires deeply invasive QEMU
patching and tight coupling to a specific SystemC version. Using Zenoh as the primary bus
and SystemC only at the Verilator boundary keeps the co-simulation path opt-in and
maintainable.

### ADR-009: KVM/hvf prohibited in slaved modes and for Cortex-M

**Decision**: Hardware virtualization is disabled whenever zenoh-clock is active and for
all Cortex-M targets.
**Rationale**: `slaved-suspend` and `slaved-icount` both require control of TCG internals
(translation block exit hooks, `qemu_icount_bias`) that are bypassed when KVM/hvf owns
execution. Cortex-M profiles are not supported by any current hypervisor; QEMU silently
falls back to TCG anyway and may misbehave with `-accel kvm` on M-profile targets.
