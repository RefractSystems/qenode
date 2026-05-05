# ADR-010: Platform Description Format & OpenUSD Alignment

## Status
Accepted

## Context
In early milestones, we implemented `repl2qemu` to parse Renode's `.repl` format. While this achieves our goal of "Renode parity", `.repl` is a bespoke format unique to one tool.

For a modern Digital Twin platform (FirmwareStudio) where physics and cyber-nodes (firmware) coexist, we need a format that is:
1. Standardized and extensible.
2. Easily parsed by numerous languages (Python, Rust, C++).
3. Designed to map 1:1 with **OpenUSD (Universal Scene Description)** primitives.

## Decision
We will adopt a custom, hierarchical **YAML format** (`.yaml`) as the primary "modern" hardware description for `VirtMCU`.

### Schema Design: The "Cyber Prim" Vision
Our YAML schema is explicitly designed to mirror a future OpenUSD schema. In USD, everything is a "Prim" (primitive) with typed "Attributes". 

A `VirtMCU` YAML platform consists of a `machine` definition and a list of `peripherals`.

```yaml
machine:
  name: flight_controller
  type: arm-generic-fdt
  cpus:
    - name: cpu0
      type: cortex-a15-arm-cpu
      memory: sysmem  # Link to the system memory container

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
    interrupts: [37]
    container: sysmem
```

### Rationale
1.  **OpenUSD Readiness**: By using a hierarchical `name`/`type`/`properties` structure, we can eventually replace the YAML parser with a USD parser (`pxr.Usd`) without changing our internal Emitter logic.
2.  **Federated Simulation Standard (FSS)**: The declarative structure of YAML enables seamless manifest generation for FSS orchestrators, detailing the exact hardware capabilities, abstraction levels, and timing requirements of the virtual MCU.
3.  **SAL/AAL Integration**: By defining peripherals strongly in YAML, we can programmatically map virtual peripheral endpoints to Sensor/Actuator Abstraction Layer transfer functions in future cyber-physical integrations.
4.  **Tooling Ecosystem**: YAML has first-class support in every major language. It allows for easy validation using JSON Schema or Pydantic.
5.  **Migration Path**: We provide a `repl2yaml` tool to ensure legacy users can instantly modernize their hardware descriptions without data loss.

## Action Plan
1.  Create `tools/repl2yaml.py`: Converts Renode `.repl` files to this new schema.
2.  Create `tools/yaml2qemu.py`: Parses the YAML and drives the existing `FdtEmitter`.
3.  Update `scripts/run.sh`: Add `--yaml` support.
