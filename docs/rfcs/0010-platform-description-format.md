# RFC-0010: Platform Description Format & OpenUSD Alignment

## Status
Accepted

## Context
In early milestones, we implemented `repl2qemu` to parse a legacy emulation configuration format. While this achieved our initial goals, this was a bespoke format unique to one tool.

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

### The "Generation vs Instantiation" Principle
Unlike Renode's `.repl` format, which is primarily for **instantiating** pre-compiled C# classes at runtime, the VirtMCU format is designed for **full-stack generation**.

| Feature | Renode (.repl) | VirtMCU (Ideal SOTA) |
| :--- | :--- | :--- |
| **Primary Role** | Runtime Instantiation | Build-time Generation |
| **Logic Source** | Manual C# Implementation | Unified IDL (TypeSpec) |
| **Firmware Sync** | Manual Headers | Auto-generated Headers |
| **Fidelity Gate** | Post-boot Error | Compile-time Verification |

We will move from starting with Silicon SVDs to starting with a **Logical Domain Model (IDL)** using TypeSpec. The `virtmcu-cli gen` tool will then produce the SVD, the C headers, the Rust QOM stubs, and the YAML schema from a single authority.

### Rationale
1.  **OpenUSD Readiness**: By using a hierarchical `name`/`type`/`properties` structure, we can eventually replace the YAML parser with a USD parser (`pxr.Usd`) without changing our internal Emitter logic.
2.  **Federated Simulation Standard (FSS)**: The declarative structure of YAML enables seamless manifest generation for FSS orchestrators, detailing the exact hardware capabilities, abstraction levels, and timing requirements of the virtual MCU.
3.  **SAL/AAL Integration**: By defining peripherals strongly in YAML, we can programmatically map virtual peripheral endpoints to Sensor/Actuator Abstraction Layer transfer functions in future cyber-physical integrations.
4.  **Tooling Ecosystem**: YAML has first-class support in every major language. It allows for easy validation using JSON Schema or Pydantic.


