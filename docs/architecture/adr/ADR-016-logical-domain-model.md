# ADR-016: Logical Domain Model for World Specification

## Status
Proposed

## Context
VirtMCU has transitioned from a flat, emulator-centric configuration format (.repl) to a hierarchical World YAML. Currently, the schema validation is fragmented:
1.  **Python (`world_schema.py`)**: Uses Pydantic for authoring validation.
2.  **Rust (`topology.rs`)**: Uses Serde for execution-time parsing.
3.  **Documentation (`Chapter 10`)**: Describes the intent but is not programmatically linked to the code.

To achieve Enterprise SOTA standards and align with our OpenUSD roadmap (ADR-010), we need a **Single Source of Truth (SSoT)**. We have selected **TypeSpec** as the IDL to define this model. 

This ADR formalizes the **Logical Domain Model**—the mathematical ontology that the TypeSpec implementation must follow.

## Decision: The VirtMCU Ontology

We define the World as a hierarchical tree of **Prims** (Primitives), Attributes, and Relationships.

### 1. Core Taxonomy (Hierarchy)

| Level | Component | OpenUSD Mapping | Description |
| :--- | :--- | :--- | :--- |
| **0** | `World` | `UsdStage` | The global simulation container. Holds global metadata (seeds, version). |
| **1** | `Node` | `UsdGeomXform` | A discrete execution entity (e.g., a car ECU or a physics engine). |
| **2** | `Machine` | `CyberPrim` | The internal compute architecture of a Node (CPUs, Arch). |
| **2** | `Resource` | `ResourcePrim` | MMIO-mapped entities (Memory, Peripherals). |
| **1** | `Topology` | `UsdRelationship` | The connectivity graph between Nodes. |

### 2. Primitive Type System

To ensure bit-fidelity between Python (Authoring) and Rust (Execution), we define strict logical types:

- **`Address` / `Size`**: Unsigned 64-bit integer (`uint64`). In YAML, these may be expressed as hex strings but must normalize to `uint64`.
- **`NodeID`**: A unique identifier. To support both legacy and modern systems, it is a `string` (which may contain a numeric representation).
- **`Protocol`**: A closed enumeration: `[Ethernet, Uart, CanFd, Spi, FlexRay, Lin, Rf802154, RfHci]`.
- **`Coordinate`**: A 3-tuple of 64-bit floats `(f64, f64, f64)`.

### 3. Entity Definitions

#### 3.1 The Node
A Node is the primary unit of parallelism. 
- **Attributes**: `id` (required), `name` (optional), `role` (Cyber \| Physics).
- **Invariants**: `id` must be unique within the `World`.

#### 3.2 The Resource (Peripheral/Memory)
- **Attributes**: `name`, `type`, `address`, `size`.
- **Properties**: A key-value map of `UsdAttribute` equivalents (e.g., `baudrate`, `vendor_id`).
- **Interrupts**: A list of `Relationship` objects connecting a resource to an IRQ controller.

#### 3.3 The Link (WireLink)
Defines a point-to-point or bus-based communication channel.
- **Attributes**: `protocol`, `nodes` (List of `NodeID`), `baud` (optional).
- **Invariant**: All `NodeID`s in the link must be present in the `World.nodes` list.

#### 3.4 The WirelessMedium
- **Attributes**: `medium_type`, `max_range_m`.
- **Relationship**: A map of `NodeID` to `Coordinate`.

### 4. Validation Invariants (The Semantic Contract)

The TypeSpec implementation must facilitate the generation of validators that enforce:
1.  **Strict Connectivity**: No message may flow between nodes unless a `Link` or `WirelessMedium` relationship exists.
2.  **No Split-Brain**: Node definitions belong exclusively to the `topology.nodes` namespace (deprecated legacy paths are rejected).
3.  **Address Normalization**: All peripheral addresses must be mapped within the `Machine` address space.

## Rationale
- **OpenUSD Readiness**: By defining our model in terms of Prims and Relationships, the transition to a `.usd` native format becomes a 1:1 mapping exercise rather than a refactor.
- **Polyglot Safety**: Code generation from this model ensures the Rust `DeterministicCoordinator` and Python `yaml2qemu` tool share the exact same understanding of the world.
- **Enterprise MBSE Alignment**: This model can be mapped to SysML v2 Block Definition Diagrams (BDDs) via automated transforms.

## Consequences
- The existing `world_schema.py` and `topology.rs` will be deprecated and replaced by generated code.
- Developers must use the TypeSpec IDL to propose changes to the hardware/topology description format.
