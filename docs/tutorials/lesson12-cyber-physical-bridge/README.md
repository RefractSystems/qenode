# Lesson 12 — The Cyber-Physical Bridge (SAL/AAL)

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
    virtmcu-time-authority  ─── Zenoh sim/clock/advance/{id}  ──► hw/rust/clock
                                                                  (TimeAuthority role)
        │  Zenoh sim/sensor/{id}/sensordata_{i}
        ▼
sensor QEMU plugin ◄─── read by firmware via MMIO
        │  Zenoh firmware/control/{id}/{actuator_id}
        ▲
actuator QEMU plugin ◄── firmware MMIO write
```

## Clock Suspend → Cyber Bridge Timing Link

The `virtmcu-time-authority` tool drives the simulation clock and synchronizes with
external physics. It sends a `ClockAdvanceReq` to QEMU and waits for a `ClockReadyResp`.
After each quantum, it collects actuator commands from Zenoh and pushes them to
physics (e.g., via shared memory).

## Two Operating Modes

### 1. Standalone — RESD Replay

For deterministic CI/CD regression testing without a physics engine:

```bash
# Terminal 1: QEMU in suspend mode
target/release/virtmcu-run --dtb board.dtb -kernel firmware.elf \
    -device virtmcu-clock,mode=suspend,node=0 -nographic -monitor none

# Terminal 2: Play the RESD trace
target/release/virtmcu-resd-replay --resd test_trace.resd --node-id 0 --delta-ns 1000000
```

### 2. Integrated — MuJoCo Zero-Copy Bridge

For closed-loop control validation with real physics:

```bash
# Terminal 1: QEMU in suspend mode
target/release/virtmcu-run --dtb board.dtb -kernel firmware.elf \
    -device virtmcu-clock,mode=suspend,node=0 -nographic -monitor none

# Terminal 2: Time Authority with MuJoCo SHM bridge
target/release/virtmcu-time-authority \
  --node-id 0 --n-sensors 6 --n-actuators 2 \
  --physics shm --delta-ns 1000000 \
  --sensor-prefix sim/sensor --topic-prefix firmware/control
```

The bridge creates a POSIX shared memory segment `/dev/shm/virtmcu_mujoco_0`. Your MuJoCo
process maps the same segment and uses the epoch-counter protocol to synchronize.

### Shared Memory Layout

```c
struct MjSharedLayout {
    uint32_t nsensordata;        // offset  0  bridge writes at init
    uint32_t nu;                 // offset  4  bridge writes at init
    uint64_t bridge_seq;         // offset  8  bridge increments; MuJoCo polls
    uint64_t mujoco_seq;         // offset 16  MuJoCo increments; bridge polls
    double   sensordata[N];      // offset 24  MuJoCo writes after mj_step
    double   ctrl[M];            // offset 24+N*8  bridge writes, MuJoCo reads
};
```

Minimal Python MuJoCo side:

```python
import mmap, ctypes, os, time

SHM_NAME = "/dev/shm/virtmcu_mujoco_0"
while not os.path.exists(SHM_NAME):
    time.sleep(0.1)

shm_fd = os.open(SHM_NAME, os.O_RDWR)
buf = mmap.mmap(shm_fd, 0)

# Read metadata
nsensordata = ctypes.c_uint32.from_buffer(buf, 0).value
nu = ctypes.c_uint32.from_buffer(buf, 4).value

bridge_seq_ptr = ctypes.c_uint64.from_buffer(buf, 8)
mujoco_seq_ptr = ctypes.c_uint64.from_buffer(buf, 16)

sensordata_ptr = (ctypes.c_double * nsensordata).from_buffer(buf, 24)
ctrl_ptr = (ctypes.c_double * nu).from_buffer(buf, 24 + nsensordata * 8)

while True:
    # Wait for bridge_seq to increment
    while mujoco_seq_ptr.value == bridge_seq_ptr.value:
        time.sleep(0.001)
    
    # 1. Read ctrl[] from bridge
    # mj_data.ctrl[:] = [ctrl_ptr[i] for i in range(nu)]
    
    # 2. Step physics
    # mujoco.mj_step(model, data)
    
    # 3. Write sensordata[] for bridge
    # for i in range(nsensordata):
    #     sensordata_ptr[i] = mj_data.sensordata[i]
        
    # 4. Signal completion
    mujoco_seq_ptr.value = bridge_seq_ptr.value
```

## Running the Smoke Test

```bash
make dev-unit
```

This runs the native unit tests for the cyber bridge and the sensor/actuator components.
