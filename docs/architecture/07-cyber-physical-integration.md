# Chapter 7: Cyber-Physical Integration

## Bridging the Gap

VirtMCU is designed specifically for **cyber-physical co-simulation**. In these systems, firmware does not exist in a vacuum; it interacts with a physical world governed by continuous time and differential equations. 

---

## 1. The Sensor/Actuator Abstraction Layer (SAL/AAL)

Firmware speaks in discrete, binary counts (ADC values, PWM duty cycles). Physics engines speak in continuous, physical quantities (acceleration, torque, voltage). SAL/AAL acts as the translation layer at the peripheral boundary.

### Actuator Path (Firmware → Physics)
Peripherals like PWM, DAC, or GPIO outputs decode firmware register writes into physical quantities. For example, a motor PWM peripheral converts a duty cycle write into an expected torque. This value is published over the simulation bus to the physics engine (e.g., MuJoCo).

### Sensor Path (Physics → Firmware)
Sensor peripherals (ADC, IMU, encoder) receive physical quantities from the physics engine and encode them into firmware-readable register values, applying configurable noise models and transfer functions.

---

## 2. Co-Simulation Hardware Integration

While SAL/AAL connects abstract physics, VirtMCU also integrates with external digital logic simulators (RTL/SystemC) using two distinct paths (detailed in Chapter 4):
- **Path A (Unix Socket Bridge)**: A lightweight, custom protocol for simple custom logic.
- **Path B (Remote Port)**: An industry-standard interface targeting Verilator models and existing Xilinx/SystemC ecosystems. It natively transports TLM-2.0 `b_transport` payloads over IPC to a Remote Port Slave implementation.

---

## 3. The "Cyber Prim" Vision (OpenUSD)

In traditional robotics simulation, there is a hard wall between the Physics Engine (geometry, joints) and the Cyber Node (firmware, registers). VirtMCU breaks this wall by treating an emulated microcontroller as a first-class **"Cyber Prim"** within the **OpenUSD (Universal Scene Description)** ecosystem.

### USD-Aligned YAML
To bridge today's ecosystem with a USD-native future, VirtMCU uses a strongly-typed YAML schema designed to map 1:1 with USD Primitives and Attributes.
- **Machine as Prim**: A `CyberNode` (Custom Prim) represents the entire MCU.
- **Peripherals as Children**: CPUs and memory regions are nested under the machine prim.
- **Relationships as Interconnects**: Interrupt lines and bus links are modeled as USD Relationships.

### The Cyber-Physical Bridge
VirtMCU acts as a compliant node in federated simulation environments (like NVIDIA Omniverse). It pauses execution, waits for the overarching orchestrator to calculate a physics frame, ingests the updated physical state, and resumes firmware execution in perfect lockstep.

---

## 3. Simulation Modes

### Integrated Mode (Live Physics)
VirtMCU connects live to a physics engine. Zero-copy shared memory or high-speed Zenoh links allow actuator outputs to be applied to the physical model before each physics step (`mj_step`), ensuring immediate physical consequences for firmware actions.

### Standalone Mode (RESD)
For CI/CD, VirtMCU can run without a live physics engine by replaying sensor values from **Renode Sensor Data (RESD)** files. This allows for deterministic testing of control logic against recorded "golden" physical traces.
