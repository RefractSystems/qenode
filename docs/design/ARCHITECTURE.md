# virtmcu Architecture

## 1. What virtmcu Is

virtmcu is a **deterministic multi-node firmware simulation framework** built on QEMU.
It is the QEMU layer of **FirmwareStudio**, a digital twin platform for embedded systems
where a physics engine (MuJoCo) simulates the physical world and acts as the master clock
for all cyber nodes.

### Binary Fidelity — the non-negotiable constraint

**The same firmware ELF that programs a real MCU must run unmodified inside VirtMCU.**

This is the highest-priority design rule. It means:
- Peripherals are mapped at the **exact** base addresses and with the **exact** register
  layouts specified in the target MCU's datasheet.
- Interrupt numbers match the physical NVIC/GIC configuration.
- Reset register values match silicon defaults.
- Co-simulation infrastructure (`zenoh-clock`, `zenoh-netdev`, `zenoh-chardev`) is
  entirely invisible to the firmware — no guest-visible MMIO, no firmware API.
- Firmware is compiled once for the MCU target. It does not know whether it is running
  on silicon or inside QEMU.

Any feature that requires the firmware to be recompiled or modified to work in VirtMCU
is a defect in VirtMCU's peripheral models or machine description, not a firmware issue.
See [ADR-006](ADR-006-binary-fidelity.md) for enforcement rules and test requirements.

### The co-simulation thesis

Firmware for cyber-physical systems cannot be tested in isolation. It reads sensors,
drives actuators, and communicates with peer microcontrollers — all of which unfold in
physical time. Correct simulation requires that every virtual MCU shares the same notion
of time, that inter-node communication is deterministically ordered by virtual time (not
wall-clock scheduling), and that the boundary between firmware registers and physical
quantities is explicitly modeled.

virtmcu addresses these requirements at the QEMU layer, using native Rust QOM modules
(and legacy C modules) linked directly into the emulator. No Python daemons run in the 
simulation loop. All new core development is **Rust-first** to leverage the language's 
memory safety and strong concurrency primitives.

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
│  ┌──────────────┐   mj_step()   ┌──────────────────┐                 │
│  │  MuJoCo      │ ────────────► │  TimeAuthority   │                 │
│  │  (physics)   │               │  (Python)        │                 │
│  │              │ ◄──────────── │                  │                 │
│  └──────────────┘  sensor data  └────────┬─────────┘                 │
│                                          │                           │
│                     Zenoh GET sim/clock/advance/{node_id}            │
│                     (no Python middleman — native Rust plugin)       │
│                                          │                           │
│              ┌───────────────────────────┼────────────────────┐      │
│              │  QEMU node 0              │  QEMU node 1       │      │
│              │  + hw/rust/               │  + hw/rust/        │      │
│              │    zenoh-clock    ◄───────┘    zenoh-clock    │      │
│              │    zenoh-netdev   ◄────────────zenoh-netdev   │      │
│              │    zenoh-chardev  ◄────────────zenoh-chardev  │      │
│              │  + QOM peripherals        │  + QOM peripherals │      │
│              │    (SAL/AAL boundary)     │    (SAL/AAL boundary)  │      │
│              │                           │                    │      │
│              │  firmware (bare-metal C)  │  firmware          │      │
│              └───────────────────────────┴────────────────────┘      │
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

**Implementation**: `hw/rust/zenoh-clock` is a native QOM device that:
1. Hooks into the TCG execution loop via the `virtmcu_tcg_quantum_hook` function pointer
   injected into `cpu-exec.c` by `patches/apply_zenoh_hook.py`.
2. At each quantum boundary, calls `cpu_exit()` to request a clean translation-block exit,
   releases the BQL, and blocks on a Zenoh `GET` to `sim/clock/advance/{node_id}`.
3. On reply, re-acquires the BQL and optionally advances `timers_state.qemu_icount_bias`
   for exact nanosecond virtual time in `slaved-icount` mode.

**Three clock modes**:

| Mode | QEMU flags | Throughput | Use when |
|---|---|---|---|
| `standalone` | (none) | **100%** | Development and CI without a physics engine. Full TCG speed. |
| `slaved-suspend` | `-device zenoh-clock,mode=suspend` | **~95%** — only TB-boundary pause | **Default.** Control loops ≥ one quantum. |
| `slaved-icount` | `-device zenoh-clock,mode=icount`<br>`-icount shift=0,align=off,sleep=off` | **~15–20%** | Firmware measures sub-quantum intervals (PWM, µs DMA). |

**BQL constraint**: The Zenoh `GET` call must always be made with the BQL released.
Blocking while holding the BQL deadlocks the QEMU process — the main event loop (QMP,
GDB stub, chardev I/O) cannot acquire the lock. 

In Rust, this is managed via RAII guards in `virtmcu-qom/src/sync.rs`:

```rust
{
    // Temporarily release BQL to block on Zenoh
    let _bql_unlock = Bql::temporary_unlock(); 
    zenoh_reply = zenoh_session.get(queryable).wait();
    // BQL is automatically re-acquired when _bql_unlock goes out of scope
}
```

### Pillar 2 — Deterministic Multi-Node Communication

In a multi-node simulation, every message crossing a node boundary must carry a virtual
timestamp. The receiving node's delivery machinery must not inject the message into the
guest until its virtual clock reaches the stamped time. This is the only way to make
inter-node behavior reproducible across runs, regardless of host scheduling.

**Ethernet** (`hw/rust/zenoh-netdev`): A custom `-netdev` backend that:
- On TX: reads the current `QEMU_CLOCK_VIRTUAL` value, attaches it as a header, publishes
  the frame to `sim/eth/frame/{src_node}/tx` on Zenoh.
- On RX: a Zenoh subscriber receives frames into a priority queue keyed by delivery
  virtual time. A `QEMUTimer` on `QEMU_CLOCK_VIRTUAL` fires when the earliest queued
  frame's timestamp is reached, injecting it into the guest NIC via `qemu_send_packet`.

**UART** (`hw/rust/zenoh-chardev`): The same virtual-timestamp model applied to serial
bytes, using QEMU's chardev backend API. Enables multi-node UART communication (e.g.,
firmware on MCU-A sends a command string to MCU-B's UART) with correct virtual ordering.
Also supports human-in-the-loop interactivity — terminal input is delivered at the correct
virtual time rather than injected at whatever wall-clock moment the user typed.

**Wire protocol** (TimeAuthority ↔ QEMU):
```
GET sim/clock/advance/{node_id}
  payload → { uint64 delta_ns; uint64 mujoco_time_ns; }           (16 bytes)
  reply   ← { uint64 current_vtime_ns; uint32 n_frames; uint32 error_code; }  (16 bytes)

error_code: 0=OK, 1=STALL (QEMU didn't reach TB boundary), 2=ZENOH_ERROR
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

**Dynamic machines** (`arm-generic-fdt` patch series & `virt` machine): Machine types that
instantiate CPUs, memory, and peripherals entirely from a Device Tree blob at runtime.
`-machine arm-generic-fdt -hw-dtb board.dtb` (for ARM) or `-machine virt -dtb board.dtb` (for RISC-V) replaces the hardcoded C machine structs.

**Dynamic QOM plugins**: `hw/` is symlinked into QEMU's source tree and compiled as proper
QEMU modules (`--enable-modules`). The resulting `.so` files are auto-discovered via
QEMU's `module_info` table. All peripherals are native C or Rust (via FFI).
Core infrastructure and all new peripherals are written in **Rust** using the `virtmcu-qom` 
safe wrapper library.

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

## 4. The MMIO Lifecycle: Firmware to Physics

Understanding how an instruction in the guest firmware ultimately results in a physical action in the simulation (or a SystemC transaction) is critical to understanding virtmcu.

Here is the exact lifecycle of a single Memory-Mapped I/O (MMIO) write:

### 1. The Guest Instruction (Firmware)
The firmware executes a standard store instruction to a hardware register:
```assembly
LDR R0, =0x40013000  // Base address of a PWM peripheral
LDR R1, =0x0000007F  // Target duty cycle value
STR R1, [R0, #0x04]  // Write to the PWM_DUTY register (offset 0x04)
```
The firmware has no knowledge of the simulator. It expects this write to change physical voltage.

### 2. The QEMU TCG Intercept (Emulator)
Because `0x40013000` is mapped as an MMIO region rather than standard RAM, QEMU's software memory management unit (`softmmu`) intercepts the write during TCG execution.

### 3. The MemoryRegion Routing (QOM)
QEMU looks up `0x40013000` in its memory tree and finds the `MemoryRegionOps` struct associated with our custom peripheral. It invokes the C-level `write` callback defined in that struct, passing the **relative offset** (`0x04`) and the data (`0x7F`).

### 4. The Language Boundary (C to Rust/SystemC)
Execution now branches depending on the peripheral's implementation:

*   **Native Rust Peripherals (`virtmcu-qom`)**: QEMU calls an `extern "C"` trampoline. The trampoline safely casts the raw C `opaque` pointer to the Rust peripheral struct (e.g., `&mut PwmDevice`) and invokes its `.write(offset, data, size)` trait method.
*   **SystemC/Verilator Models (`mmio-socket-bridge`)**: The write lands in the Rust `mmio-socket-bridge`. The bridge serializes the offset and data into a binary packet and sends it over a UNIX socket to the `systemc_adapter` process. The QEMU vCPU thread **blocks** (safely yielding the BQL via `Bql::temporary_unlock()`) until the SystemC TLM-2.0 transaction completes and an ACK is returned over the socket, ensuring perfect temporal synchronization.

### 5. Zenoh Serialization & Dispatch (SAL/AAL)
Inside the Rust peripheral's `.write()` method, the device updates its internal state. Because this state change affects the physical world (an Actuator), it must notify the physics engine:
1. It retrieves the current exact virtual time via `qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL)`.
2. It serializes the new duty cycle and the virtual timestamp into a binary payload (e.g., FlatBuffers).
3. It dispatches the payload via Zenoh: `self.publisher.put(payload).wait();`

The message is routed to `sim/actuator/pwm/0`, where the physics engine (MuJoCo) applies the torque at the exact virtual microsecond it was commanded.

---

## 5. Concurrency, Safety, and the BQL

The Big QEMU Lock (BQL) is the primary synchronization mechanism in the emulator. 
VirtMCU enforces strict safety rules to prevent deadlocks and race conditions.

### RAII BQL Management
Direct FFI calls to `virtmcu_bql_lock/unlock` are discouraged. Instead, Rust plugins use 
RAII guards from `virtmcu-qom`:
- `Bql::lock()`: Acquires the BQL and returns a `BqlGuard`.
- `Bql::temporary_unlock()`: If the BQL is held, releases it and returns a `BqlUnlockGuard` 
  that re-acquires it on drop.

### Threading Model
- **VCPU Threads**: Execute guest instructions. Hold BQL most of the time. Must release BQL 
  before blocking on external I/O (Zenoh, sockets).
- **Background Threads**: (e.g., Zenoh subscribers). Must NEVER block waiting for the BQL. 
  Instead, they push data to lock-free queues and notify the guest via `QEMUTimer` or 
  `virtmcu_cond_signal`.

### Planned: `wait_yielding_bql`
To simplify the common pattern of a VCPU thread blocking on a condition while holding a 
peripheral-specific mutex, we are implementing `wait_yielding_bql`. This pattern ensures 
the BQL is released while waiting and re-acquired before returning, maintaining the 
required lock order: `BQL -> Peripheral Mutex`.

---

## 6. Development Standards & Determinism

VirtMCU adheres to enterprise-grade engineering standards to ensure 100% reproducible 
simulations.

### The "Zero-Sleep" Mandate
Usage of `std::thread::sleep` or `time.sleep` is strictly prohibited in simulation hot 
paths and integration tests. Deterministic synchronization is achieved via:
- **Zenoh Liveliness**: `wait_for_zenoh_discovery` ensures peers are ready.
- **Virtual Time**: `VirtualTimeAuthority` drives the simulation clock.
- **UART Signaling**: `wait_for_line_on_uart` confirms guest readiness.

### Bifurcated Testing Strategy
We employ a two-layer testing approach:
1. **White-Box (Rust)**: Native `cargo test` suites in `hw/rust/` validate internal state 
   machines, FFI boundaries, and protocol parsing without booting QEMU.
2. **Black-Box (Python)**: `pytest` suites in `tests/` orchestrate full QEMU nodes, 
   Zenoh routers, and physics engines to verify end-to-end system behavior.

---

## 7. QEMU Build Details

### Version and Patches

- **Base**: QEMU 11.0.0 (git tag `v11.0.0`)
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

### Rust and Zenoh Dependencies

Core plugins are now written in native Rust (located in `hw/rust/`). 
Rust dependencies, including the `zenoh` crate, are managed by `cargo` and statically linked into the resulting QEMU modules (`.so` files). 

This eliminates the previous dependency on the external `zenoh-c` shared library and removes the need for complex `LD_LIBRARY_PATH` configurations to load the plugins.

---

## 8. Timing Design and Performance

> **See also:** [docs/TIME_MANAGEMENT_DESIGN.md](TIME_MANAGEMENT_DESIGN.md) — sequence diagrams, Big QEMU Lock mechanics, clock mode selection, and virtual-time test automation in one place.

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

## 9. Prior Art

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

## 10. Build Environments

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

## 11. Architectural Decision Records

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

---

## 12. AI and Advanced Observability (Phase 12 & 13)

As virtmcu evolves from a foundational emulator into a robust digital twin environment, observability and AI accessibility become first-class concerns.

### Advanced Observability (COOJA-Inspired)
FirmwareStudio needs rich, interactive observability (visual timelines, network topologies, interactive virtual boards). virtmcu provides this without embedding a GUI into QEMU by:
1. Tracing CPU sleep states and peripheral events via `hw/rust/zenoh-telemetry` and publishing deterministic timelines over Zenoh.
2. Enabling dynamic manipulation of network latency and drop rates via RPC endpoints on the `zenoh_coordinator`.
3. Emitting UI state (LEDs, Buttons) via SAL/AAL abstraction topics.

### AI Debugging & MCP Interface
To support LLM-driven debugging and lifecycle management, virtmcu includes a standalone **Model Context Protocol (MCP)** server (`tools/mcp_server/`).
- **Control**: AI agents can provision boards, flash firmware, and control node lifecycle (start/stop/pause).
- **Introspection**: AI agents can inspect raw memory, registers, and disassemble code dynamically via the `qmp_bridge.py` wrapper.
- **I/O Integration**: Agents can interact with UART consoles and monitor network state.
*(For more details, see `docs/MCP_DESIGN.md`)*.

---

## 13. Common Pitfalls & Troubleshooting

### SysBus Mapping vs. `-device` (The arm-generic-fdt Trap)
A frequent point of confusion for developers migrating from standard QEMU machines is why a device added via the `-device` command line option is not accessible to the guest firmware (resulting in Data Aborts).

**The Cause**: In the `arm-generic-fdt` machine, QEMU uses the Device Tree as the source of truth for both instantiation *and* memory mapping. If you add a device via `-device`, QEMU will instantiate the object, but it will **not** automatically map its MMIO regions into the guest's physical address space. Mapping only occurs if a corresponding node exists in the DTB with a `reg` property.

**The Fix**: Always declare your peripherals in the platform YAML. The `yaml2qemu.py` tool will ensure that both the DTB node is created (mapping the device) and the corresponding `-device` argument is either handled by QEMU's FDT loader or added to the CLI.

### `mmio-socket-bridge` Address Offsets
The `mmio-socket-bridge` (and most other virtmcu bridges) delivers **offsets relative to the region base**, not absolute physical addresses. 

**The Cause**: This follows standard QEMU `MemoryRegionOps` behavior. If a bridge is mapped at `0x10000000` and the guest performs a read at `0x10000004`, the `addr` field in the `mmio_req` packet will be `0x00000004`.

**Adapter Contract**: Adapters receive pure relative offsets and must NOT add the base address back. The `addr` field in `mmio_req` is always `guest_PA - region_base`, as QEMU computes this before invoking the `MemoryRegionOps` callback.

### Zenoh Router Reachability
If QEMU hangs at startup or `TimeAuthority` reports a "Timeout" during `sim/clock/advance`, first verify that the Zenoh router is reachable from the QEMU container.

- **Check `ZENOH_ROUTER`**: Ensure the `router=` property on `zenoh-clock` matches your router's endpoint.
- **Status Codes**: Check the `status` field in the `ClockReadyPayload`. A status of `1` (`ZCLOCK_STATUS_STALL_TIMEOUT`) indicates that QEMU reached the router but failed to advance instructions fast enough to hit the next quantum boundary.

If you are new to QEMU, SystemC, physics simulators (like MuJoCo), or Zenoh, the `virtmcu` codebase can seem intimidating because it glues all these domains together. Here is how you should approach learning the system:

### 1. Start with the Tutorials
Do not read the C code first. Go to the `tutorial/` folder and work through the lessons in order.
- **Lessons 1 & 2** teach you how QEMU works (Device Trees, QOM, and Memory-Mapped I/O). You will learn that QEMU is just a giant event loop that translates ARM assembly into x86 assembly (TCG) and routes memory reads/writes to C functions (peripherals).
- **Lessons 5 & 9** teach you SystemC. You will learn that SystemC is just a C++ library with a cooperative threading model and a simulation clock, used by hardware engineers to model buses (like CAN or I2C) before they are manufactured.
- **Lesson 7** teaches you Zenoh. You will learn that Zenoh is a Pub/Sub message bus (like MQTT or ROS2) but heavily optimized for Rust and C.

### 2. Understand the Trade-offs (Pros/Cons)
Whenever you see a design choice in `virtmcu`, look for an ADR (Architecture Decision Record) in the `docs/` folder.
For example, **ADR-011** explains exactly why we use Zenoh instead of standard TCP/UDP sockets (standard sockets ruin determinism because the host OS network stack introduces random latency).
**ADR-010** explains why we use YAML instead of Renode's `.repl` format (YAML maps cleanly to OpenUSD, the industry standard for 3D physics scenes).

### 3. The "No Python in the Loop" Rule
You will notice a lot of C and Rust code in `hw/zenoh/` and `tools/systemc_adapter/`. Why didn't we just write a simple Python script to connect QEMU to MuJoCo?
Because Python's Global Interpreter Lock (GIL) and garbage collector introduce milliseconds of latency. If a simulated drone motor controller (running at 1000 Hz) has to wait for a Python script to forward a message to the physics engine every 1 millisecond, the simulation will run slower than real-time. By writing native C plugins (`.so` files) that load directly into QEMU's address space, we achieve near-native performance. Python is strictly reserved for *offline* tooling (like generating the Device Tree in `tools/yaml2qemu.py` or running the test suite).

### 4. Where to Ask for Help
If a QEMU macro like `OBJECT_DECLARE_SIMPLE_TYPE` confuses you, look at `hw/dummy/dummy.c`. We intentionally keep a heavily commented "dummy" peripheral in the tree as a learning template. Never copy-paste complex QEMU upstream code without understanding it; start from the dummy device and build up.

## 14. Related Reference Documents
* [Zenoh Topic Map](ZENOH_TOPIC_MAP.md) - A definitive map of all Zenoh channels/topics in the federation.
* [Timing Model](TIMING_MODEL.md) - How virtual time is synchronized.
