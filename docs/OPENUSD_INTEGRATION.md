# OpenUSD & virtmcu: The "Cyber Prim" Vision

This document outlines the architectural vision for integrating **virtmcu** hardware emulators natively into the **OpenUSD (Universal Scene Description)** ecosystem.

---

## 1. The Vision: Unified Digital Twins

In traditional robotics and industrial simulation, there is a hard wall between the **Physics Engine** (geometry, joints, kinematics) and the **Cyber Node** (firmware, registers, interrupts).

- **Physics** lives in `.usd`, `.urdf`, or `.mjcf`.
- **Cyber** lives in `.repl`, `.dts`, or hardcoded C structs.

**virtmcu** breaks this wall. Our goal is to treat an ARM microcontroller not as an external process, but as a first-class **"Cyber Prim"** inside the USD scene graph. 

Imagine a single `.usd` file where:
- A drone's chassis is a `Xform` prim.
- Its motors are `Physics` prims.
- Its flight controller is a `CyberNode` prim.

---

## 2. Our Intermediate Standard: USD-Aligned YAML

To bridge today's ecosystem with tomorrow's USD-native future, virtmcu uses a **strongly-typed YAML schema** designed to map 1:1 with USD Primitives and Attributes.

### Why YAML first?
- **Lightweight**: No need for the 500MB `pxr` USD library for headless CI or simple firmware testing.
- **Ubiquitous**: Easily edited by humans and parsed by every language in the simulation loop (Python, Rust, C++).
- **Extensible**: Perfectly mirrors the hierarchical nature of USD.

### The Schema Concept
A virtmcu YAML platform is structured as a tree of "Objects":

```yaml
# A CyberNode represents the entire "machine"
machine:
  name: flight_controller
  type: arm-generic-fdt
  cpus:
    - name: cpu0
      type: cortex-a15-arm-cpu
      memory: sysmem  # USD Relationship: links to the memory prim

# Peripherals are children of the CyberNode
peripherals:
  - name: sram
    type: qemu-memory-region
    address: 0x40000000
    size: 0x08000000
    properties:
      ram: true
    container: sysmem

  - name: uart0
    type: pl011
    address: 0x09000000
    interrupts: 
      - gic@37 # USD Relationship: links to the interrupt controller prim
    container: sysmem
```

---

## 3. Mapping to OpenUSD Primitives

When virtmcu transitions to native USD support, the mapping will be direct:

| virtmcu YAML Concept | OpenUSD Concept | Attributes |
| :--- | :--- | :--- |
| `machine` | `CyberNode` (Custom Prim) | `machineType`, `cpuCount` |
| `cpu` | `Processor` (Custom Prim) | `cpuModel`, `frequency` |
| `peripheral` | `Peripheral` (Custom Prim) | `address`, `size`, `type` |
| `interrupts` | `Relationship` | `target`, `line` |
| `properties` | `Attributes` | (Any typed value) |

---

## 4. Federated Simulation and The Cyber-Physical Bridge

Integrating a digital twin into a broader simulation ecosystem (like NVIDIA Omniverse) requires rigorous standardization of sensor and actuator data. virtmcu aligns with the **Accellera Federated Simulation Standard (FSS)** and the concept of the **Cyber-Physical Bridge**.

1. **SAL/AAL (Sensor/Actuator Abstraction Layer)**: Peripherals in virtmcu do not ingest raw USD attributes directly. The Abstraction Layer translates continuous physics data (e.g., a floating-point joint velocity from Omniverse) into discrete binary register values, applying noise profiles and ADC quantization.
2. **Federated Orchestration**: virtmcu acts as a compliant FSS node. It pauses execution, waits for the overarching simulation orchestrator to calculate the next frame of physics, ingests the updated USD stage properties, and resumes execution.

## 5. Technical Benefits for the USD Community

1.  **Non-Destructive Composition**: Using USD "Layers" and "Overrides", a developer can take a base "STM32F4" Cyber Prim and non-destructively add a custom "FPGA Accelerator" peripheral for a specific project.
2.  **Semantic Search**: Tools like NVIDIA Omniverse can query the entire simulation stage to find all "UART" devices, regardless of whether they are part of a car, a robot, or a factory sensor.
3.  **Unified Time Master**: As defined in **ADR-001**, QEMU advances its virtual clock only when the USD-native physics master (e.g., MuJoCo or OmniPhysX) grants a quantum, ensuring firmware and physics are always in sync.

---

## 6. Current Implementation Status

- [x] **Parser**: `tools/yaml2qemu.py` converts our USD-aligned YAML into QEMU Device Trees.
- [x] **Migration**: `tools/repl2yaml.py` converts legacy Renode `.repl` files into this modern standard.
- [x] **Runner**: `scripts/run.sh` supports `.yaml` natively.
- [ ] **SAL/AAL Framework**: (Phase 10) Base classes for translating physical properties into binary data.
- [ ] **Native USD / FSS Plugin**: (Phase 10+) Native `pxr::Usd` and Accellera FSS ingestion.

---

## 7. How to Use

To boot a machine defined in this future-proof format:

```bash
./scripts/run.sh --yaml my_platform.yaml --kernel my_firmware.elf -nographic
```

To modernize an existing Renode project:

```bash
python3 tools/repl2yaml.py legacy_board.repl --out modern_board.yaml
```
