# Lesson 10 — The Cyber-Physical Bridge (SAL/AAL)

This lesson explains how virtmcu creates a causal, deterministic link between
firmware running in QEMU and an external physics engine or prerecorded data stream.
The mechanism is the **Sensor/Actuator Abstraction Layer (SAL/AAL)**.

## Why SAL/AAL?

A virtual MCU sees the world entirely through MMIO registers. A physics engine (or
prerecorded telemetry) speaks in continuous state variables — angular velocity,
temperature, joint torque. The SAL/AAL is the translation layer:

- **SAL** (Sensor Abstraction Layer): Reads physical states from the simulation
  (or from an RESD file), models sensor noise/calibration, and injects data into
  the MMIO register file at the exact virtual time the firmware would sample them.
- **AAL** (Actuator Abstraction Layer): Intercepts MMIO writes from firmware
  (e.g., a PWM duty cycle), translates them into physical command semantics
  (e.g., target torque), and forwards them to the physics engine.

## Architecture

```
MuJoCo / RESD replay
        │  shared memory (mjData) or file I/O
        ▼
tools/cyber_bridge/
    mujoco_bridge  ─── Zenoh sim/clock/advance/{id}  ──► hw/rust/clock
    resd_replay                                            (TimeAuthority role)
        │  Zenoh sim/sensor/{id}/{name}
        ▼
mmio-socket-bridge ◄─── read by firmware via MMIO ──► SAL peripheral model
        │  Zenoh sim/actuator/{id}/{name}
        ▲
firmware MMIO write
```

## Clock Suspend → Cyber Bridge Timing Link

Task 7.7 added `VirtmcuQuantumTiming` to `include/virtmcu/hooks.h` and wired it
into `clock.c`. The `ClockAdvancePayload.mujoco_time_ns` field carries the
physics-engine simulation time to QEMU, where `virtmcu-clock` stores it in
`s->mujoco_time_ns`. QEMU-internal SAL models can then call:

```c
VirtmcuQuantumTiming t;
virtmcu_get_quantum_timing(&t);
// t.quantum_start_vtime_ns + fraction * t.quantum_delta_ns = interpolated_ns
// t.mujoco_time_ns = MuJoCo time at the start of this quantum
```

External tools (`mujoco_bridge`, `resd_replay`) receive `current_vtime_ns` in the
Zenoh reply and send `mujoco_time_ns` in the next request, keeping the loop closed.

## Two Operating Modes

### 1. Standalone — RESD Replay

For deterministic CI/CD regression testing without a physics engine:

```bash
# Terminal 1: QEMU in suspend mode
scripts/run.sh --dtb board.dtb -kernel firmware.elf \
    -device virtmcu-clock,mode=suspend,node=0 -nographic -monitor none

# Terminal 2: Play the RESD trace (acts as TimeAuthority)
tools/cyber_bridge/build/resd_replay test_trace.resd 0
# Optional: override quantum size to 500µs:
tools/cyber_bridge/build/resd_replay test_trace.resd 0 500000
```

`resd_replay` terminates automatically when virtual time exceeds the last sample
timestamp, leaving QEMU blocked at the next quantum boundary.

### 2. Integrated — MuJoCo Zero-Copy Bridge

For closed-loop control validation with real physics:

```bash
# Terminal 1: QEMU in suspend mode
scripts/run.sh --dtb board.dtb -kernel firmware.elf \
    -device virtmcu-clock,mode=suspend,node=0 -nographic -monitor none

# Terminal 2: MuJoCo bridge (node_id=0, nu=2 actuators, nsensordata=6, 1ms quanta)
tools/cyber_bridge/build/mujoco_bridge 0 2 6 1000000
```

The bridge creates a POSIX shared memory segment `/virtmcu_mujoco_0`. Your MuJoCo
process maps the same segment and fills `sensordata[6]` before each `mj_step()`.
The bridge reads those values and publishes them to
`sim/sensor/0/sensordata_{0..5}`.

### Shared Memory Layout

```c
struct MjSharedLayout {        /* at offset 0 in /virtmcu_mujoco_{node_id} */
    uint32_t nsensordata;      /* written by bridge on startup */
    uint32_t nu;               /* written by bridge on startup */
    uint64_t mujoco_time_ns;   /* written by MuJoCo each step */
    double sensordata[nsensordata]; /* written by MuJoCo, read by bridge */
    double ctrl[nu];                /* written by bridge (Zenoh), read by MuJoCo */
};
```

Minimal Python MuJoCo side:

```python
import mmap, ctypes, os, mujoco

SHM_NAME = "/virtmcu_mujoco_0"
shm_fd = os.open(f"/dev/shm{SHM_NAME}", os.O_RDWR)
buf = mmap.mmap(shm_fd, 0)

model = mujoco.MjModel.from_xml_path("robot.xml")
data  = mujoco.MjData(model)

while True:
    mujoco.mj_step(model, data)

    # Write mujoco_time_ns and sensordata into shared memory
    offset = 0
    ctypes.c_uint32.from_buffer(buf, offset).value = model.nsensordata
    ctypes.c_uint32.from_buffer(buf, offset + 4).value = model.nu
    offset += 8
    ctypes.c_uint64.from_buffer(buf, offset).value = int(data.time * 1e9)
    offset += 8
    for i in range(model.nsensordata):
        ctypes.c_double.from_buffer(buf, offset).value = data.sensordata[i]
        offset += 8

    # Read ctrl from shared memory
    for i in range(model.nu):
        data.ctrl[i] = ctypes.c_double.from_buffer(buf, offset).value
        offset += 8
```

## C++ Abstraction Interfaces

All backends implement the interfaces in `tools/cyber_bridge/include/virtmcu/sal_aal.hpp`:

```cpp
namespace virtmcu {

class Sensor {
public:
    virtual std::string          get_name() const = 0;
    virtual std::vector<double>  get_reading(uint64_t vtime_ns) = 0;
};

class Actuator {
public:
    virtual std::string  get_name() const = 0;
    virtual void         apply_command(uint64_t vtime_ns,
                                       const std::vector<double>& values) = 0;
};

class SimulationBackend {
public:
    virtual bool  init() = 0;
    virtual void  step_to(uint64_t vtime_ns) = 0;
    virtual void  register_sensor(Sensor*)   = 0;
    virtual void  register_actuator(Actuator*) = 0;
};

} // namespace virtmcu
```

## Building `cyber_bridge`

```bash
cd tools/cyber_bridge
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j$(nproc)
# Produces:
#   build/resd_replay   — RESD-file TimeAuthority
#   build/mujoco_bridge — shared-memory MuJoCo bridge
```

Prerequisites: `zenoh-c` built in `third_party/zenoh-c/`, CMake ≥ 3.14,
a C++17 compiler.

## OpenUSD / YAML Address Map Tool

Generate a C++ header from your board YAML so the SAL/AAL C++ code always uses
the same base addresses as the firmware Device Tree:

```bash
tools/usd_to_virtmcu.py boards/my_robot.yaml > include/board_addresses.hpp
```

Output:
```cpp
namespace virtmcu { namespace address_map {
    constexpr uint64_t MEMORY_BASE = 0x40000000;
    constexpr uint64_t UART0_BASE  = 0x09000000;
    constexpr uint64_t IMU0_BASE   = 0x50000000; // mmio-socket-bridge
} }
```

Use `IMU0_BASE` in your `MuJoCoSensor` or `ResdSensor` subclass to guarantee
the SAL model reads from the same address the firmware writes to.

## Running the Smoke Test

```bash
bash tests/fixtures/guest_apps/cyber_bridge/smoke_test.sh
```

Expected output:
```
[cyber_bridge] Building tools/cyber_bridge...
[cyber_bridge] TEST 1: OpenUSD Metadata Tool...
[cyber_bridge] TEST 1 PASSED — generated constexpr address map
[cyber_bridge] TEST 2: RESD Parser — sample parsing...
[cyber_bridge] TEST 2 PASSED — RESD parser correctly reads samples
[cyber_bridge] TEST 3: resd_replay startup + empty-file rejection...
[cyber_bridge] TEST 3 PASSED — resd_replay rejects missing file
[cyber_bridge] TEST 4: mujoco_bridge shared memory creation...
[cyber_bridge] TEST 4 PASSED — mujoco_bridge created shm segment

=== Cyber Bridge smoke test PASSED ===
```

## Debugging Tips

**RESD replay exits immediately with "No sensor channels found"**
- Check the RESD file magic: `xxd test.resd | head -2` — should start with `52455344` (`RESD`).
- Verify the block's `data_size` field is large enough to include the subheader,
  metadata size field, and at least one sample. Use the formula:
  `data_size = subheader_bytes + 8 + metadata_bytes + N_samples * bytes_per_sample`

**Firmware reads stale sensor values**
- Check that `resd_replay` is publishing to `sim/sensor/{node_id}/{name}`.
  Monitor with: `python3 -c "import zenoh, sys; s=zenoh.open(zenoh.Config()); s.declare_subscriber('sim/sensor/**', lambda s,_: sys.stdout.write(str(s.key_expr) + '\n'))"`
- The SAL peripheral on the QEMU side must subscribe to the same topic.

**`mujoco_bridge` exits with "Timeout waiting for QEMU"**
- Verify QEMU started with `-device virtmcu-clock,mode=suspend,node={same_id}`.
- Check the Zenoh router is reachable from both processes.
- Run `zenoh-router` locally: `cargo install zenohd && zenohd`

**`mujoco_bridge` ctrl[] values not reaching MuJoCo**
- Verify the MuJoCo process is mapping the correct shm segment name and reading
  at the correct byte offset (`sizeof(MjSharedLayout) + nsensordata * 8`).
- Use `xxd /dev/shm/virtmcu_mujoco_0 | head -4` to inspect the live segment.
