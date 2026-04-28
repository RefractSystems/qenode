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
- Co-simulation infrastructure (`clock`, `netdev`, `chardev`) is
  entirely invisible to the firmware — no guest-visible MMIO, no firmware API.
- Firmware is compiled once for the MCU target. It does not know whether it is running
  on silicon or inside QEMU.

Any feature that requires the firmware to be recompiled or modified to work in VirtMCU
is a defect in VirtMCU's peripheral models or machine description, not a firmware issue.
See [ADR-006](ADR-006-binary-fidelity.md) for enforcement rules and test requirements.

### The co-simulation thesis

Firmware for cyber-physical systems cannot be tested in isolation. It reads sensors,
drives actuators, and communicates with peer microcontrollers — all of which unfold in
physical time. Correct simulation requires three guarantees simultaneously:

1. **Temporal correctness**: every virtual MCU shares the same notion of time, gated by an external physics master clock.
2. **Global determinism**: two runs with identical inputs (topology, firmware, seed) produce bit-identical output — same UART bytes, same PCAP log, same actuator commands — regardless of host CPU load.
3. **Causal ordering**: inter-node communication is ordered by virtual time, not wall-clock scheduling. Same-timestamp messages are broken by canonical `(vtime, src_node_id, seq)` order enforced by a central coordinator barrier, not by OS scheduling.

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
┌─────────────────────────────────────────────────────────────────────────────┐
│  FirmwareStudio World                                                       │
│                                                                             │
│  ┌──────────────┐  mj_step()  ┌───────────────────────────────────────┐    │
│  │  MuJoCo      │ ──────────► │  TimeAuthority (Python)               │    │
│  │  (physics)   │             │  - steps all node clocks              │    │
│  │              │ ◄────────── │  - pushes topology updates            │    │
│  └──────────────┘ sensor data └───────┬───────────────────────────────┘    │
│                                       │                                     │
│               clock: ClockSyncTransport (Unix socket / Zenoh GET)          │
│               one channel per node — direct, no coordinator in path        │
│                                       │                                     │
│           ┌───────────────────────────┼──────────────────────┐             │
│           │  QEMU node 0              │   QEMU node 1        │             │
│           │  hw/rust/backbone/clock ◄─┘   hw/rust/backbone/cl│             │
│           │  hw/rust/comms/netdev         netdev             │             │
│           │  hw/rust/comms/chardev        chardev            │             │
│           │  QOM peripherals               QOM peripherals    │             │
│           │  firmware (bare-metal C)       firmware           │             │
│           └───────────┬───────────────────────────┬──────────┘             │
│                       │  emulated comms            │                        │
│                       ▼   (all inter-node traffic) ▼                        │
│            ┌─────────────────────────────────────────┐                     │
│            │  DeterministicCoordinator               │                     │
│            │  - owns topology graph (from YAML)      │                     │
│            │  - quantum barrier: waits for ALL nodes │                     │
│            │  - DataTransport (Unix / Zenoh)         │                     │
│            │  - sorts: (vtime_ns, src_id, seq_num)   │                     │
│            │  - applies topology mask (RF/wire)      │                     │
│            │  - emits PCAP log side-channel          │                     │
│            └─────────────────────────────────────────┘                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Two separate channels serve distinct purposes:**

1. **Clock sync** (TimeAuthority ↔ each QEMU node): Uses `ClockSyncTransport` — a Unix
   domain socket for single-host runs (1–3 µs RTT), or a Zenoh Queryable for distributed
   multi-host deployments (10–50 µs RTT). This channel is strictly 1:1, never routed
   through the coordinator.

2. **Emulated network traffic** (QEMU ↔ QEMU): All inter-node Ethernet frames, UART
   bytes, CAN-FD messages, RF packets, and sensor/actuator data route through the
   `DeterministicCoordinator`. The coordinator buffers all messages from quantum Q,
   waits for all nodes to signal completion, sorts them into canonical order, and delivers
   them before releasing nodes to start quantum Q+1. Virtual timestamps enforce causal
   ordering within this barrier.

---

## 3. The Five Pillars

### Pillar 1 — Cooperative Time Slaving

QEMU's virtual clock must not free-run in a cyber-physical simulation. If firmware on MCU-A
writes a PWM value at virtual time T=5 ms, the physics engine must model that output at
T=5 ms — not at whatever wall-clock moment the QEMU process happened to execute that
instruction. This requires QEMU to be a **time slave**: it runs at full TCG speed within
each quantum but blocks at every quantum boundary until the external TimeAuthority grants
the next advance.

**Implementation**: `hw/rust/backbone/clock` is a native QOM device that:
1. Hooks into the TCG execution loop via the `virtmcu_tcg_quantum_hook` function pointer
   injected into `cpu-exec.c` by `patches/apply_zenoh_hook.py`.
2. At each quantum boundary, calls `cpu_exit()` to request a clean translation-block exit,
   releases the BQL, and blocks on a reply from the TimeAuthority (using `ClockSyncTransport`).
3. On reply, re-acquires the BQL and optionally advances `timers_state.qemu_icount_bias`
   for exact nanosecond virtual time in `slaved-icount` mode.

**Three clock modes**:

| Mode | QEMU flags | Throughput | Use when |
|---|---|---|---|
| `standalone` | (none) | **100%** | Development and CI without a physics engine. Full TCG speed. |
| `slaved-suspend` | `-device clock,mode=suspend` | **~95%** — only TB-boundary pause | **Default.** Control loops ≥ one quantum. |
| `slaved-icount` | `-device clock,mode=icount`<br>`-icount shift=0,align=off,sleep=off` | **~15–20%** | Firmware measures sub-quantum intervals (PWM, µs DMA). |

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
timestamp AND be delivered in a globally canonical order. Virtual timestamps alone are
insufficient: two messages with identical timestamps from different nodes produce
non-deterministic delivery order if processed directly by independent Zenoh subscribers
(OS scheduling breaks ties differently each run).

**The PDES barrier model**: The `DeterministicCoordinator` implements the Parallel
Discrete Event Simulation (PDES) barrier pattern:

1. Each node sends all outbound messages to the coordinator (tagged with virtual timestamp
   and a per-source sequence number).
2. Each node signals "quantum Q done" to the coordinator.
3. The coordinator waits until **all N nodes** have signaled completion.
4. The coordinator sorts all buffered messages by `(delivery_vtime_ns, source_node_id,
   sequence_number)` — a total order that is deterministic and independent of arrival order.
5. The coordinator applies the topology mask (drops messages not permitted by the declared
   link graph).
6. The coordinator delivers sorted messages to recipients and signals "quantum Q+1 start".

This guarantees that two runs with identical inputs produce identical delivery sequences.

**Ethernet** (`hw/rust/comms/netdev`): On TX, attaches a `ZenohFrameHeader` (vtime + seq)
and publishes to the coordinator's ingress topic. On RX, a priority queue keyed by delivery
virtual time feeds a `QEMUTimer` on `QEMU_CLOCK_VIRTUAL`.

**UART** (`hw/rust/comms/chardev`): Same virtual-timestamp model for serial bytes.

**Topology declaration** (world YAML `topology:` section): The full network graph —
node IDs, wire links, RF neighborhoods, max wireless range — is declared at startup.
The coordinator loads this graph and rejects connections not in the graph. Mobile node
positions are pushed by the physics engine before each quantum; the coordinator
recomputes RF neighborhoods deterministically from position vectors.

**Wire protocol** (TimeAuthority ↔ QEMU, via `ClockSyncTransport`):
```
Advance request  (TA → QEMU):  16 bytes: [uint64 delta_ns LE][uint64 mujoco_time_ns LE]
Ready reply      (QEMU → TA):  16 bytes: [uint64 vtime_ns LE][uint32 n_frames LE][uint32 error_code LE]

error_code: 0=OK, 1=STALL (QEMU didn't reach TB boundary), 2=TRANSPORT_ERROR

Transport options (ClockSyncTransport implementations):
  ZenohClockTransport    — Zenoh Queryable; default for multi-host deployments
  UnixSocketClockTransport — Unix domain socket; default for single-host (~10 nodes)
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

**Status**: Implemented (Phase 10). Base classes and native Zenoh support are fully 
integrated into the core simulation loop.

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

### Pillar 6 — Global Determinism Guarantee

**Invariant**: Two simulation runs with identical world YAML, firmware ELFs, and
`global_seed` produce byte-identical UART output, identical PCAP log, and identical
actuator command sequences — regardless of host CPU load, thread scheduling, or
wall-clock time.

This invariant is enforced at three levels:

**Level 1 — Clock**: The TimeAuthority drives all virtual time. Nodes cannot advance
time on their own. The clock channel (`ClockSyncTransport`) is strictly 1:1 between TA
and each QEMU node; no intermediary touches the clock messages.

**Level 2 — Message ordering**: The `DeterministicCoordinator` enforces canonical
`(vtime_ns, src_node_id, seq_num)` order for all inter-node messages within each quantum.
This eliminates OS-scheduling-dependent tie-breaking.

**Level 3 — Stochastic seeding**: All protocols with random behavior (CSMA/CA backoff,
BLE advertising slot selection) MUST use `virtmcu_api::seed_for_quantum(global_seed,
node_id, quantum_number)` as the sole PRNG seed source.
 The `global_seed` comes from
the world YAML `topology.global_seed` field (default: 0). Using system time, PIDs, or
`rand::thread_rng()` as a seed in any simulation code path is banned.

**Observability**: The coordinator emits a libpcap-format log (`--pcap-log <path>`)
containing every inter-node message with its virtual timestamp. Two runs with the same
seed produce byte-identical PCAP files. This file is the ground-truth determinism oracle
for CI regression tests and the feed for the Wireshark extcap plugin.

**Scaling**: Single-host scenarios (~10 nodes) use Unix socket clock transport and a
single-process coordinator — no Zenoh router required. Distributed scenarios (1000s of
nodes across hosts) use hierarchical coordinators connected via Zenoh, with one
coordinator per host cluster. The per-cluster coordinator protocol is identical; only
the inter-cluster transport changes.

---

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

### Threading Model
- **VCPU Threads**: Execute guest instructions. Every vCPU has its own thread in QEMU. MMIO 
  handlers (read/write callbacks) execute in the context of a VCPU thread.
- **Main Loop Thread**: Manages QMP, GDB, and asynchronous I/O.
- **Peripheral Threads**: Peripherals may spawn background threads (e.g., Zenoh 
  subscribers, heartbeats).

**Crucial Invariant**: Only ONE thread can hold the BQL at any time. MMIO handlers 
and QEMUTimer callbacks are invoked by QEMU with the BQL **already held**.

### `BqlGuarded<T>` vs. `Mutex<T>`
In standard Rust, shared state is protected by `std::sync::Mutex<T>`. However, because 
most peripheral code runs under the BQL, a `Mutex` is redundant and misleading — it 
is permanently uncontended because the BQL already serializes access.

VirtMCU mandates the use of `BqlGuarded<T>` for state accessed from MMIO handlers, 
timers, and `SafeSubscriber` callbacks.
- **Prohibited**: `std::sync::Mutex<T>` in peripheral state structs (unless marked 
  with `// MUTEX_EXCEPTION` for cross-thread sync with non-BQL background threads).
- **Required**: `BqlGuarded<T>` for all BQL-protected state. It uses `UnsafeCell<T>` 
  internally and debug-asserts that the BQL is held at every access point.

### RAII BQL Management
Direct FFI calls to `virtmcu_bql_lock/unlock` are discouraged. Instead, Rust plugins use 
RAII guards from `virtmcu-qom`:
- `Bql::lock()`: Acquires the BQL and returns a `BqlGuard`.
- `Bql::temporary_unlock()`: If the BQL is held, releases it and returns a `BqlUnlockGuard` 
  that re-acquires it on drop. Use this before any blocking call (e.g., Zenoh `GET` or 
  UNIX socket read).

---

## 6. Multi-Node Communication: A Step-by-Step Example

To understand how time, data, and threads interact, consider two VirtMCU nodes (A and B) 
communicating over a virtual UART bus.

### Scenario: Node A sends 'X' to Node B

1.  **Firmware write (Node A)**: Node A's firmware writes 'X' to its UART TX register.
2.  **MMIO Intercept**: Node A's VCPU thread enters the `chardev` write handler 
    (holding Node A's BQL).
3.  **Timestamping**: `chardev` reads Node A's virtual clock (e.g., T=100.0ms).
4.  **Zenoh Dispatch**: Node A serializes the byte and timestamp into a 
    `ZenohFrameHeader` + payload. It publishes to `sim/uart/A/tx`.
5.  **Zenoh Federation**: The message travels over the Zenoh bus to Node B's subscriber.
6.  **Reception (Node B)**: A Zenoh background thread in Node B receives the message. 
    It **cannot** touch Node B's guest state because it does not hold Node B's BQL.
7.  **Priority Queue**: Node B's subscriber pushes the message into its `local_heap` 
    (protected by `BqlGuarded`). It updates a QEMUTimer to fire at T=100.0ms + 
    propagation delay.
8.  **Time Advancement**: Node B's VCPU thread executes until its virtual clock reaches 
    the timer threshold.
9.  **Timer Callback**: Node B's timer fires. QEMU invokes the `rx_timer_cb` (holding 
    Node B's BQL).
10. **Guest Injection**: `rx_timer_cb` pops 'X' from the heap and calls 
    `qemu_chr_be_write`, which triggers a UART RX interrupt in Node B's firmware.

This process ensures that even if Node A runs much faster than Node B on the host CPU, 
Node B sees the data at the exact virtual moment Node A intended.

### Peripheral Time and Concurrency (The Architecture Plan)

When a peripheral like a UART or an 802.15.4 radio processes data, the CPU is not frozen; it continues to execute instructions concurrently. However, physical hardware takes time to shift bits over a wire or through the air.

Currently, virtmcu employs a **simple immediate execution model**: if firmware writes to a UART, the MMIO writes are processed instantly in virtual time. This causes a "flooding" effect where bytes are sent with nearly identical virtual timestamps, violating physical baud rate constraints.

**Critique of the Simple Model & Hazards to Mitigate**:
Before moving to a realistic model, we must account for several edge cases that the simple model ignores:
1.  **FIFO Drain Rates**: Real UARTs and radios have hardware FIFOs. Backpressure isn't just toggling a TX flag per byte; it requires modeling the continuous drain of the FIFO at the configured baud rate.
2.  **RX Reception Delay**: When a Zenoh packet arrives from the network at $T_{arrival}$, it takes $T_{duration}$ for the bits to physically clock into the receiving peripheral before the RX interrupt should assert.
3.  **Lifecycle and Reset Hazards**: If a transmission timer is pending and the firmware resets the peripheral or disables the TX block, the timer **must** be cancelled (`virtmcu_timer_del`). Failing to do so results in spurious interrupts triggering in the future.
4.  **Baud Rate Volatility**: Firmware might change the baud rate register while bytes are in flight. The delay model must lock the duration at the start of a byte's transmission.
5.  **Jitter in Slaved-Suspend**: In the default `slaved-suspend` mode, timers will fire at the exact virtual nanosecond, but the CPU's instruction execution catches up in blocks. For cycle-accurate bit-banging, the system relies on `slaved-icount` mode.

**The Planned Path Forward (Phase 29: Fidelity & Backpressure)**:
*(For an exhaustive evaluation of modeling options and industry comparisons, see [PERIPHERAL_TIMING_EVALUATION.md](PERIPHERAL_TIMING_EVALUATION.md))*

Virtmcu prioritizes **Software-Observable Fidelity over Cycle-Accuracy**. We explicitly accept the loss of microscopic bus-contention accuracy to maintain the execution speed required for CI/CD workflows. To achieve this sweet spot, all time-sensitive peripherals will transition to a realistic backpressure model using native QEMU virtual timers:
1.  **TX Backpressure**: When firmware writes to the TX FIFO, the peripheral calculates the transmission duration (e.g., `baud_delay = 10 bits / 115200 bps = 86.8 µs`). It schedules a `QEMUTimer` (tied to `QEMU_CLOCK_VIRTUAL`) to fire at `now + baud_delay`.
2.  **No Zenoh Clock Subscription**: Individual peripherals **do not** subscribe to the Zenoh clock. The `clock` device synchronizes the global `QEMU_CLOCK_VIRTUAL`. Peripherals rely purely on local QEMU timers.
3.  **Timer Callbacks**: When the timer fires, the peripheral pops the byte from the TX FIFO, transmits it over Zenoh, updates the "FIFO Full/Empty" flags, and asserts the TX interrupt. If the FIFO is not empty, it re-arms the timer for the next byte.
4.  **RX Modeling**: Incoming Zenoh frames are queued. A timer is scheduled to simulate the physical reception delay before drip-feeding the bytes into the guest's RX FIFO.

This planned mechanism naturally throttles the guest firmware to the correct virtual baud rate without artificially freezing the CPU or adding complex network subscriptions to individual plugins.

---

## 7. Data Formatting and Serialization

All data sent over Zenoh must be deterministic and cross-platform.

### Rules for Wire Protocols
1.  **No `to_ne_bytes()`**: Always use `to_le_bytes()` or `to_be_bytes()` for explicit 
    endianness.
2.  **FlatBuffers**: Use FlatBuffers for complex structures (e.g., FlexRay frames, 
    Telemetry) to ensure schema evolution safety and zero-copy performance.
3.  **Fixed Headers**: Simple protocols (UART, SPI, Ethernet) use the `ZenohFrameHeader` 
    defined in `virtmcu-api`.

### Prohibited Patterns
- **Direct Pointer Copies**: Never use `ptr::copy_nonoverlapping` to serialize Rust 
    structs to the wire. Padding and layout differences between compilers can break 
    determinism.
- **Raw Transmutation**: `mem::transmute` of structs is banned for I/O. Use `.pack()` 
    and `.unpack()` methods.

---

## 8. QEMU Build Details

### Version and Patches

- **Base**: QEMU 11.0.0 (git tag `v11.0.0`)
- **Patches applied in order by `scripts/setup-qemu.sh`**:
  1. `patches/arm-generic-fdt-v3.mbx` — 33-patch series (patchew ID
     `20260402215629.745866-1-ruslichenko.r@gmail.com`), applied via `git am`
  2. `patches/apply_zenoh_hook.py` — AST-injects `virtmcu_tcg_quantum_hook` function
     pointer into `accel/tcg/cpu-exec.c`
  3. `patches/apply_zenoh_netdev.py` — registers the Zenoh netdev backend
  4. `patches/apply_zenoh_chardev.py` — registers the Zenoh chardev backend

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

With `--enable-modules`, this produces `hw-transport-zenoh.so`, `hw-transport-unix.so`, `hw-virtmcu-dummy.so`, etc.,
installed in `QEMU_MODDIR`. `QEMU_MODULE_DIR` is set by `scripts/run.sh`.

### Rust Dependencies

Core plugins are now written in native Rust (located in `hw/rust/`). 
Rust dependencies, including the `zenoh` crate and `DataTransport` implementations, are managed by `cargo` and statically linked into the resulting QEMU modules (`.so` files). 

This eliminates the previous dependency on the external `zenoh-c` shared library and removes the need for complex `LD_LIBRARY_PATH` configurations to load the plugins.

---

## 9. Timing Design and Performance

> **See also:** [TIME_MANAGEMENT_DESIGN.md](TIME_MANAGEMENT_DESIGN.md) — sequence diagrams, Big QEMU Lock mechanics, clock mode selection, and virtual-time test automation in one place.

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

## 10. Prior Art

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

## 11. Build Environments

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

## 12. Architectural Decision Records

### ADR-001: Three clock modes (standalone / slaved-suspend / slaved-icount)

**Decision**: Implement three distinct clock modes rather than a single unified approach.
**Rationale**: `slaved-icount` is required for sub-quantum timer precision but costs 5–8×
throughput. Making it the default would unnecessarily penalize the 95% of workloads that
do not need nanosecond accuracy within a quantum. `standalone` mode is essential for
development and CI without a physics engine.

### ADR-002: Zenoh for emulated network traffic; `ClockSyncTransport` for clock

**Decision**: Use Zenoh pub/sub (via the `DeterministicCoordinator`) for emulated network
traffic (Ethernet frames, UART bytes, RF packets, sensor/actuator data). Use the
`ClockSyncTransport` abstraction (Unix socket for single-host, Zenoh Queryable for
multi-host) for the clock-advance RPC. These are NOT the same channel.

**Rationale**: Emulated network traffic benefits from Zenoh's language-agnostic pub/sub,
flexible topology routing, and cross-host capability. Clock sync requires the lowest
possible latency, strict 1:1 semantics, and no async executor involvement — properties
that conflict with Zenoh's Queryable model (see Zenoh executor deadlock postmortem,
2026-04-18, and ADR-015). Topology is declared in YAML and enforced by the coordinator;
runtime discovery via Zenoh multicast scouting is disabled in simulation mode.

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

### ADR-014: Global Determinism as a First-Class Requirement

**Decision**: Any simulation run with identical world YAML, firmware ELFs, and
`global_seed` MUST produce bit-identical output. Violations are bugs in VirtMCU.

**Rationale**: Firmware developers rely on reproducible simulation to debug rare
concurrent faults. Non-determinism caused by OS scheduling (Zenoh subscriber callback
order, thread wakeup order) makes it impossible to reproduce bugs. The PDES coordinator
barrier and canonical message ordering are the minimum necessary mechanism.

**Constraints**:
- Topology is declared, not discovered. The `DeterministicCoordinator` owns the graph.
- All stochastic seeding uses `seed_for_quantum(global_seed, node_id, quantum_number)`.
- The PCAP log is the observable determinism oracle for CI.

### ADR-015: `ClockSyncTransport` — Separate Clock from Comms Transport

**Decision**: The clock-advance RPC (TimeAuthority ↔ QEMU) uses a dedicated transport
abstraction (`ClockSyncTransport` trait) separate from the emulated network transport.
For single-host deployments, the default implementation is Unix domain socket. For
multi-host, the Zenoh Queryable implementation is used.

**Rationale**: The clock path has fundamentally different requirements from pub/sub
networking: it is strictly 1:1, both endpoints are known at startup, requires the
lowest possible latency, and must never involve a broker as a SPOF. The Zenoh Queryable
executor-deadlock postmortem (`2026-04-18`) was caused by conflating the clock's blocking
RPC semantics with Zenoh's async executor model. The `ClockSyncTransport` trait decouples
these concerns and enables a Unix socket fast path (1–3 µs vs 10–50 µs).

**Interface**:
```rust
pub trait ClockSyncTransport: Send + Sync + 'static {
    fn recv_advance(&self, timeout: Duration) -> Option<ClockAdvanceReq>;
    fn send_ready(&self, resp: ClockReadyResp) -> Result<(), ClockTransportError>;
    fn shutdown(&self);
}
```

### ADR-005: SystemC for co-simulation, not for the main simulation kernel

**Decision**: SystemC TLM-2.0 is used for co-simulation with external Verilated models
(Phase 5, 9) but is not the top-level simulation kernel.
**Rationale**: SystemC as a kernel (qbox / MINRES approach) requires deeply invasive QEMU
patching and tight coupling to a specific SystemC version. Using Zenoh as the primary bus
and SystemC only at the Verilator boundary keeps the co-simulation path opt-in and
maintainable.

### ADR-009: KVM/hvf prohibited in slaved modes and for Cortex-M

**Decision**: Hardware virtualization is disabled whenever clock is active and for
all Cortex-M targets.
**Rationale**: `slaved-suspend` and `slaved-icount` both require control of TCG internals
(translation block exit hooks, `qemu_icount_bias`) that are bypassed when KVM/hvf owns
execution. Cortex-M profiles are not supported by any current hypervisor; QEMU silently
falls back to TCG anyway and may misbehave with `-accel kvm` on M-profile targets.

### ADR-016: Transport-Agnostic Data Plane

**Decision**: Abstract the data plane transport layer (`DataTransport` trait) to allow peripherals to operate over multiple communication backends (Zenoh or Unix Domain Sockets) without changing peripheral logic.

**Rationale**: While Zenoh is the preferred federation bus for distributed simulation, Unix Domain Sockets provide a zero-configuration, lower-latency alternative for single-host co-simulations and CI environments where a Zenoh router may not be present or desired. Decoupling the peripheral logic from the underlying pub/sub implementation improves maintainability and enables easier integration with other transport protocols in the future.

**Constraints**:
- All peripherals MUST use the `DataTransport` trait for I/O.
- The transport is chosen at runtime based on the `topology.transport` field in the world YAML.
- `SafeSubscription` handles BQL management and lifecycle regardless of the underlying transport.

---

## 13. AI and Advanced Observability (Phase 12 & 13)

As virtmcu evolves from a foundational emulator into a robust digital twin environment, observability and AI accessibility become first-class concerns.

### Advanced Observability (COOJA-Inspired)
FirmwareStudio needs rich, interactive observability (visual timelines, network topologies, interactive virtual boards). virtmcu provides this without embedding a GUI into QEMU by:
1. Tracing CPU sleep states and peripheral events via `hw/rust/observability/telemetry` and publishing deterministic timelines over Zenoh.
2. Enabling dynamic manipulation of network latency and drop rates via RPC endpoints on the `deterministic_coordinator`.
3. Emitting UI state (LEDs, Buttons) via SAL/AAL abstraction topics.

### AI Debugging & MCP Interface
To support LLM-driven debugging and lifecycle management, virtmcu includes a standalone **Model Context Protocol (MCP)** server (`tools/mcp_server/`).
- **Control**: AI agents can provision boards, flash firmware, and control node lifecycle (start/stop/pause).
- **Introspection**: AI agents can inspect raw memory, registers, and disassemble code dynamically via the `qmp_bridge.py` wrapper.
- **I/O Integration**: Agents can interact with UART consoles and monitor network state.
*(For more details, see [MCP_DESIGN.md](MCP_DESIGN.md))*.

---

## 14. Common Pitfalls & Troubleshooting

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

- **Check `ZENOH_ROUTER`**: Ensure the `router=` property on `clock` matches your router's endpoint.
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

## 15. Related Reference Documents
* [Zenoh Topic Map](ZENOH_TOPIC_MAP.md) - A definitive map of all Zenoh channels/topics in the federation.
* [Timing Model](TIMING_MODEL.md) - How virtual time is synchronized.
