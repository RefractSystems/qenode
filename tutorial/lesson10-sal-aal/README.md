# Lesson 10 — The Cyber-Physical Bridge (SAL/AAL)

This lesson explores how virtmcu creates a causal link between firmware running in QEMU and external physics engines (or prerecorded data streams). We accomplish this by utilizing the **Sensor/Actuator Abstraction Layer (SAL/AAL)**.

## What are SAL and AAL?
The virtual MCU views the world entirely through Memory-Mapped I/O (MMIO) registers. The real physical world operates on continuous state variables (e.g., angular velocity, temperature, joint torque).

- **SAL (Sensor Abstraction Layer)**: Reads physical states (e.g., `IMU(x, y, z)`) from the physics simulation via Zenoh, adds any necessary sensor noise/calibration models, and packs them into the exact bit-layout of the hardware peripheral's registers.
- **AAL (Actuator Abstraction Layer)**: Listens for MMIO register writes from the firmware (e.g., a PWM duty cycle), translates that into physical command semantics (e.g., Target RPM), and publishes it out via Zenoh to the physics engine.

## Two Operating Modes

`virtmcu` supports two modes of injecting telemetry data into the firmware:

### 1. Standalone (RESD Replay)
For fast, deterministic CI/CD regression testing, you don't want to spin up a full physics simulator. Instead, you can use the **Renode Sensor Data (RESD)** binary format. 
We provide a C++ `resd_replay` executable (in `tools/cyber_bridge`) that parses an RESD file and acts as the TimeAuthority, advancing the QEMU clock and pushing deterministic sensor readings exactly when the firmware expects them.

```bash
# Terminal 1: Start QEMU in suspend mode
scripts/run.sh --dtb board.dtb -kernel firmware.elf -device zenoh-clock,mode=suspend,node=0

# Terminal 2: Play the RESD trace
./tools/cyber_bridge/build/resd_replay test_trace.resd 0
```

### 2. Integrated (MuJoCo Zero-Copy Bridge)
When developing advanced control algorithms, you need true Closed-Loop Simulation. QEMU and the Physics Engine (MuJoCo) run in lock-step.
To avoid Zenoh serialization overhead for massive state vectors, we use the **Zero-Copy Bridge** (`mujoco_bridge`), which uses shared memory mapping of MuJoCo's `mjData` structure. The C++ bridge coordinates the Zenoh time quantum handshake but reads/writes directly from `mjData->sensordata` and `mjData->ctrl`.

## C++ Abstraction Interfaces

The tools inside `tools/cyber_bridge` use standard C++ interfaces to separate the physics data source (RESD or MuJoCo) from the specific sensor/actuator logic.

```cpp
class Actuator {
    virtual void apply_command(uint64_t vtime_ns, const std::vector<double>& values) = 0;
};

class Sensor {
    virtual std::vector<double> get_reading(uint64_t vtime_ns) = 0;
};
```

## OpenUSD Metadata Generation

If your physical robot is defined in OpenUSD (or our custom YAML format), you don't need to manually synchronize MMIO addresses between Python/C++ and the Device Tree.
Use the `usd_to_virtmcu.py` tool to generate C++ header definitions directly from your board schema:

```bash
./tools/usd_to_virtmcu.py board.yaml > board_addresses.hpp
```
This guarantees your C++ SAL/AAL models always bind to the exact base addresses compiled into your firmware!
