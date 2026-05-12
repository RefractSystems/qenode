# Cyber-Physical Integration

## Bridging the Gap

VirtMCU is designed specifically for **cyber-physical co-simulation**. In these systems, firmware does not exist in a vacuum; it interacts with a physical world governed by continuous time and differential equations. 

To bridge this gap, we rely on the architecture introduced in the System Overview: The **Cyber Node** (currently QEMU) must communicate seamlessly with the **Physical Node** (currently MuJoCo or Omniverse) over the **Transport Layer** (currently Zenoh or Unix Sockets).

---

## 1. The Sensor/Actuator Abstraction Layer (SAL/AAL)

Firmware speaks in discrete, binary counts (ADC values, PWM duty cycles). Physical Nodes speak in continuous, physical quantities (acceleration, torque, voltage). SAL/AAL acts as the translation layer at the peripheral boundary within the Cyber Node.

### Actuator Path (Firmware → Physics)
Peripherals like PWM, DAC, or GPIO outputs decode firmware register writes into physical quantities. For example, a motor PWM peripheral inside QEMU converts a duty cycle write into an expected torque. This value is published over the Transport Layer (e.g., Zenoh) to the Physical Node (e.g., MuJoCo).

### Sensor Path (Physics → Firmware)
Sensor peripherals (ADC, IMU, encoder) receive physical quantities from the Physical Node via the Transport Layer and encode them into firmware-readable register values, applying configurable noise models and transfer functions.

---

## 2. Co-Simulation Hardware Integration

While SAL/AAL connects abstract physics, the Cyber Node also integrates with external digital logic simulators (RTL/SystemC) using two distinct paths (detailed in Communication Protocols):
- **Path A (Unix Socket Bridge)**: A lightweight Transport Layer for simple custom logic.
- **Path B (Remote Port)**: An industry-standard interface targeting Verilator models and existing Xilinx/SystemC ecosystems. It natively transports TLM-2.0 `b_transport` payloads over IPC to a Remote Port Slave implementation.

---

## 3. The "Cyber Prim" Vision (OpenUSD)

In traditional robotics simulation, there is a hard wall between the Physical Node (geometry, joints) and the Cyber Node (firmware, registers). VirtMCU breaks this wall by treating an emulated microcontroller as a first-class **"Cyber Prim"** within the **OpenUSD (Universal Scene Description)** ecosystem.

### USD-Aligned YAML
To bridge today's ecosystem with a USD-native future, VirtMCU uses a strongly-typed YAML schema designed to map 1:1 with USD Primitives and Attributes.
- **Machine as Prim**: A `CyberNode` (Custom Prim) represents the entire MCU.
- **Peripherals as Children**: CPUs and memory regions are nested under the machine prim.
- **Relationships as Interconnects**: Interrupt lines and bus links are modeled as USD Relationships.

### The Cyber-Physical Bridge
The Cyber Node acts as a compliant participant in federated simulation environments (like NVIDIA Omniverse). It pauses execution, waits for the Physical Node orchestrator to calculate a physics frame, ingests the updated physical state via the Transport Layer, and resumes firmware execution in perfect lockstep.

---

## 4. Simulation Modes

### Integrated Mode (Live Physics)

The Cyber Node connects live to a Physics Engine via the **Physics Gateway**
(`virtmcu-physics-gateway`). The gateway is a dedicated process that sits between the
Time Authority and the physics engine:

- The Time Authority collects all actuator commands for a completed quantum, serialises
  them as a `PhysicsTrigger` FlatBuffer, and forwards them to the gateway.
- The gateway writes actuator values to a shared-memory region (`/dev/shm`) and signals
  the physics engine via a Linux futex doorbell.
- The physics engine runs one time-step, writes updated sensor values back to SHM, and
  acknowledges via the same futex mechanism.
- The gateway reads the sensor values, publishes them to Zenoh, and returns
  `PhysicsDone` to the Time Authority.

The Time Authority does not issue the next `ClockAdvanceReq` until `PhysicsDone` is
received, ensuring strict causal ordering across every quantum.

For architecture details, SHM layout, wire protocol, and deployment topology, see
[Physics Gateway](./12-physics-gateway.md).

### Standalone Mode (RESD)
For CI/CD, the Cyber Node can run without a live Physics Engine by replaying sensor
values from **Renode Sensor Data (RESD)** files. This allows for deterministic testing
of control logic against recorded "golden" physical traces.
