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

### Evaluation: Why YAML currently, and when do we transition?

**Pros of keeping YAML right now:**
- **Zero-Bloat CI/CD (The biggest factor)**: The official Python OpenUSD library (`pxr`) is massive (often 500MB+ depending on the build). Keeping YAML allows our core simulation loop and CI pipelines to remain incredibly lightweight. We avoid forcing developers to install heavy 3D VFX libraries just to test bare-metal firmware.
- **Human Readability & Tooling**: YAML is natively understood by Python, Rust, and developers. It's much easier to quickly hand-edit a YAML file during active development than to write a Python script using the `pxr` API to mutate a `.usd` binary cache.
- **Agnostic Core**: It keeps the emulator (`qemu` and our Rust plugins) entirely decoupled from the complexities of 3D scene graphs.

**Cons of keeping YAML:**
- **Lack of Native Composition**: OpenUSD's greatest superpower is "Composition" (Sublayers, References, Variants). With YAML, we lack native merging logic if we want modular hardware definitions (e.g., overlaying a `camera_sensor` on a `base_drone`).
- **The "Two Source of Truth" Problem**: For mobile nodes, the YAML defines the initial layout, but the physics engine (e.g., MuJoCo) moves them. If they aren't using the exact same file format, we have to synchronize the YAML state with the physics engine state continuously.
- **No Native Schema Enforcement**: OpenUSD has strict, compiled schema validation. With YAML, we rely on custom Python parsing logic to manually check for typos or invalid node links.

**The Horizon: When will we *have* to move to OpenUSD?**
We will hit a hard ceiling with YAML when we transition from **"Firmware Testing"** to **"Federated Digital Twins"** (Phase 10+). Specifically, OpenUSD becomes mandatory when:
1. **Full Physics Integration**: When a robotic arm's firmware reads joint encoders directly from a 3D physics engine in real-time. Maintaining a separate YAML topology for the network and a USD topology for the physics becomes unmanageable.
2. **Complex System-of-Systems**: Simulating a swarm of 100 drones, where each drone has variants. OpenUSD's composition engine is mandatory for scaling this without duplicating 100 YAML files.

**The Architectural Boundary:**
Even when that horizon arrives, we will **not** rewrite `virtmcu` to ingest `.usd` directly in the hot loop. The OpenUSD orchestrator will use the USD API to compose the scene, and then **export/generate** the YAML + DTB just-in-time for the emulator to consume (as prototyped in `tools/usd_to_virtmcu.py`).

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
- [x] **SAL/AAL Framework**: (Phase 10) Base classes for translating physical properties into binary data.
- [ ] **Native USD / FSS Plugin**: (Phase 10+) Native `pxr::Usd` and Accellera FSS ingestion.

---

## 7. Commands

To boot a machine defined in this future-proof format:

```bash
./scripts/run.sh --yaml my_platform.yaml --kernel my_firmware.elf -nographic
```

To modernize an existing Renode project:

```bash
python3 tools/repl2yaml.py legacy_board.repl --out modern_board.yaml
```

To test the YAML tooling:
```bash
pytest tests/test_yaml2qemu.py
```

## 8. Project Structure

- `tools/yaml2qemu.py`: Main parser connecting YAML files to the QEMU Device Tree compiler.
- `tools/repl2yaml.py`: Migrator script converting `.repl` files to `.yaml`.
- `tools/usd_to_virtmcu.py`: Stub/Draft for future direct `.usd` ingestion.
- `test/phase3.5/`: Shell scripts verifying the YAML end-to-end boot process.

## 9. Code Style

- Python code handling the schema must use strict typing and explicitly define dictionary shapes using `TypedDict` or `dataclasses`.
- The YAML output itself must prioritize readability: use inline arrays for short lists and explicit blocks for complex mappings.

## 10. Testing Strategy

- **Parser Validation**: `tests/test_yaml2qemu.py` unit tests all edge cases of the schema (missing containers, malformed relationships).
- **End-to-End Boot**: `test/phase3.5/smoke_test.sh` executes the emitted DTB under QEMU and asserts the correct bare-metal kernel output.

## 11. Boundaries

- **Always do**: Keep the YAML schema strictly 1:1 mappable to OpenUSD concepts (Prims, Attributes, Relationships).
- **Ask first**: Before adding QEMU-specific implementation details into the YAML. The YAML should describe the *hardware*, not the emulator.
- **Never do**: Never introduce a mandatory `pxr` (OpenUSD) library dependency into the core `virtmcu` runtime. The Python USD library is massive; virtmcu must remain lightweight via the YAML bridge for CI/CD usage.
