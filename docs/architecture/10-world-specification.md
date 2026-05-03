# Chapter 10: World Specification

The **VirtMCU World YAML** (often referred to as the "World Manifest") is the single source of truth for a deterministic simulation. It serves as the digital blueprint for both the internal hardware of a node and the network topology connecting multiple nodes.

This chapter defines the schema, semantics, and execution policies of the World YAML, aligned with the **Cyber-Physical Prim** vision (ADR-010).

---

## 1. Overview: The Digital Twin Blueprint

A "World" in VirtMCU represents a complete, self-contained simulation environment. Unlike traditional emulators that rely on command-line arguments to build a machine, VirtMCU uses a declarative manifest to ensure:
1.  **Reproducibility**: The same YAML always produces the same hardware and topology.
2.  **Determinism**: Simulation parameters (seeds, barriers) are baked into the manifest.
3.  **Scalability**: Machines can be effortlessly replicated and networked.

---

## 2. Root Structure

The World YAML is a hierarchical document with four primary top-level keys:

| Key | Purpose | Required |
| :--- | :--- | :--- |
| `machine` | High-level CPU and machine type configuration. | Yes (for Cyber Nodes) |
| `peripherals` | List of MMIO-mapped hardware devices. | Yes |
| `memory` | List of RAM/Flash memory regions. | Yes |
| `topology` | Multi-node connectivity and PDES settings. | Optional |

---

## 3. The `machine` Section

The `machine` block defines the heart of the simulated node.

```yaml
machine:
  name: test_board
  type: arm-generic-fdt  # The QEMU machine type
  cpus:
    - name: cpu0
      type: cortex-a15
      memory: sysmem     # Pointer to the system memory container
```

- **`type`**: Currently, `arm-generic-fdt` is the standard for ARM-based MCU simulation, allowing dynamic machine construction via Device Tree.
- **`cpus`**: A list of processor cores. Each CPU requires a `type` (e.g., `cortex-m4`, `cortex-a15`) and a reference to its primary address space (`memory`).

---

## 4. Hardware Resources: `memory` and `peripherals`

VirtMCU distinguishes between raw memory regions and active peripherals, though both are mapped into the guest's address space.

### Memory Regions
```yaml
memory:
  - name: sram
    address: 0x40000000
    size: 0x01000000
```
- **`address`**: Base address in guest memory (supports hex strings or integers).
- **`size`**: Total size of the region.

### Peripherals
Peripherals are mapped to QEMU Object Model (QOM) types.

```yaml
peripherals:
  - name: uart0
    type: UART.PL011
    address: 0x09000000
    interrupts: [37]
    properties:
      baudrate: 115200
```
- **`type`**: The peripheral type name. VirtMCU supports both native QOM names and legacy aliases for compatibility.
- **`interrupts`**: A list of IRQ numbers or connections (e.g., `gic@37`).
- **`properties`**: A dictionary of key-value pairs passed directly to the QOM object during instantiation. These are the "Attributes" in the Cyber Prim model.

---

## 5. The `topology` Section

The `topology` block defines how this node interacts with others in a Parallel Discrete Event Simulation (PDES).

### Node Identities
```yaml
topology:
  nodes:
    - name: 0  # Node IDs are typically numeric for deterministic routing
    - name: 1
```

### Wired Links (WireLink)
WireLinks define point-to-point or bus-based connections using specific protocols.

```yaml
topology:
  links:
    - type: uart
      nodes: [0, 1]
      baud: 115200
```
- **`type`**: Protocol selector (options: `uart`, `eth`, `canfd`, `flexray`, `lin`, `spi`, `rf802154`, `rfhci`).
- **`nodes`**: The IDs of the nodes participating in this link.

### Wireless Mediums (WirelessMedium)
Wireless connectivity is calculated based on Euclidean distance and a maximum range.

```yaml
topology:
  wireless:
    medium: wifi_6ghz
    max_range_m: 50.0
    nodes:
      - name: 0
        initial_position: [0.0, 0.0, 0.0]
      - name: 1
        initial_position: [10.0, 5.0, 0.0]
```

### Global Simulation Parameters
- **`global_seed`**: (int) Used to derive all node-local PRNG seeds.
- **`transport`**: (`zenoh` or `unix`) The physical layer for simulation traffic.
- **`max_messages_per_node_per_quantum`**: (int) A safety barrier to prevent infinite loops or message floods from stalling the coordinator.

---

## 6. Execution Policies

### "Topology Declared, Not Discovered"
VirtMCU enforces a strict "No Scouting" policy. All inter-node traffic must be authorized by a `link` or `wireless` entry in the World YAML. The `DeterministicCoordinator` rejects any message that does not follow a declared path.

### Split-Brain Prevention
To ensure a clean migration from legacy formats, VirtMCU rejects YAMLs that attempt to define nodes in multiple sections. Node IDs must be declared exclusively within `topology.nodes`.

### Address Normalization
While the YAML supports multiple address formats, `yaml2qemu.py` normalizes all addresses to 64-bit integers before emitting the Device Tree. Peripherals mapped without an address are treated as "floating" and must be manually attached to a parent bus (e.g., SPI sub-nodes).

---

## 7. OpenUSD Alignment and Renode `.repl` Parity

The VirtMCU World YAML is designed to map directly to **OpenUSD (Universal Scene Description)** primitives. In a future USD schema, the YAML structure translates as follows:
- `machine`, `peripherals`, `memory` → `UsdGeomXform` (or custom `CyberPrim`) hierarchical nodes.
- `properties` → `UsdAttribute`.
- `interrupts`, `links` → `UsdRelationship` (defining connections between Prims).

### Renode `.repl` Parity
To guarantee backwards compatibility, the World Schema supports all semantics from Renode's `.repl` format:
- **Ranged Interrupts**: Supported via `properties` or specific string formats in the `interrupts` list (e.g., matching Renode's `[0-3] -> nvic@[19-22]`).
- **`using` Directives**: Renode allows `#include`-style `.repl` nesting. While YAML does not natively support this, VirtMCU tooling merges included schemas during the `repl2yaml` migration phase.
- **`sysbus: init:` Tags**: Renode uses these to mock dummy memory regions early in boot. VirtMCU replicates this by instantiating `qemu-memory-region` peripherals with appropriate sizes based on the tag bounds.

---

## 8. Comparison with Legacy Formats

| Feature | VirtMCU YAML | Legacy .repl | OpenUSD (Future) |
| :--- | :--- | :--- | :--- |
| **Parsing** | Python (PyYAML) / Rust (Serde) | Custom C# / Python Regex | `pxr.Usd` API |
| **Topology** | Native `topology:` block | None (Manual scripts) | Network Prims / Graphs |
| **Hierarchy** | USD Prim Alignment | Flat list | First-class namespaces |
| **Validation** | Pydantic / JSON Schema | Runtime failures | USD Schema validation |

---

## 8. The SOTA Pipeline: Schema as Code

To maintain Enterprise-grade quality across a polyglot codebase, VirtMCU adopts a **Single Source of Truth (SSoT)** architecture. The World Specification is not manually implemented in Python or Rust; instead, it is defined using **TypeSpec**, a modern Interface Definition Language (IDL).

### The Generation Pipeline
1.  **Source of Truth**: The logical model is defined in `schema/world/main.tsp`.
2.  **IDL Compilation**: The TypeSpec compiler (`tsp`) validates the model and emits a strictly-typed JSON Schema.
3.  **Polyglot Synthesis**:
    -   **Python**: `datamodel-codegen` transforms the JSON Schema into Pydantic v2 models used by `yaml2qemu.py`.
    -   **Rust**: A corresponding generator produces Serde-compatible structs for the `DeterministicCoordinator`.
    -   **IDE Support**: The generated JSON Schema provides real-time autocompletion and validation for developers writing World YAMLs in VSCode or JetBrains IDEs.

This pipeline ensures that a change to the `Protocol` enumeration or a new attribute on a `Resource` is instantly and consistently propagated across the entire simulation stack, eliminating "split-brain" bugs between the authoring tools and the simulation engine.

---

## 9. Case Studies: The Schema in Practice

To truly grasp the implications of the World Specification, we must step beyond theoretical data structures and observe how these abstractions govern the applied reality of modern engineering. As students of cyber-physical systems, you must recognize that a schema is not merely a configuration file; it is the mathematical ontology of the world you are building. Let us examine two distinct paradigms.

### Case Study A: The Industry 4.0 Smart Factory
Consider the challenge faced by Dr. Elena Rostova, a Lead Roboticist designing an automated gigafactory. Her facility relies on a fleet of fifty Autonomous Guided Vehicles (AGVs) that transport materials between stationary, six-axis robotic welding arms. The entire ballet is orchestrated by a central Programmable Logic Controller (PLC) over a congested factory WiFi network. 

Elena cannot test her fleet collision avoidance algorithms on physical hardware without risking millions of dollars in equipment damage. Instead, she turns to the VirtMCU World Schema. For Elena, the `machine` block is merely the starting point—defining the Cortex-R5 processors inside her AGVs. Her true focus is the `topology.wireless` block. She defines a 6GHz `wifi` medium and assigns each AGV an `initial_position` in a 3D coordinate space. 

Because the VirtMCU schema is strictly deterministic, Elena can inject a specific `global_seed`. When AGV #12 and AGV #44 attempt to transmit telemetry to the PLC simultaneously, the resulting packet collision and subsequent backoff algorithm play out identically on every simulation run. Furthermore, looking toward the OpenUSD future, Elena's schema translates directly into `UsdGeomXform` primitives. Her AGV cyber-nodes are mathematically bound to their physical 3D CAD models, allowing her automation pipeline to visualize a physics-accurate, bit-exact digital twin of the factory floor before a single piece of concrete is poured.

### Case Study B: The Software-Defined Vehicle (SDV) Architect
Now, contrast the spatial challenges of robotics with the topological density of a modern automobile. Marcus Vance is the Chief SDV Architect for a next-generation electric vehicle. His vehicle is fundamentally a "data center on wheels," utilizing a Zonal Architecture: a high-performance Central Compute node communicates with four Zonal Controllers via gigabit Automotive Ethernet, which in turn interface with legacy actuators via CAN-FD.

For Marcus, spatial positioning is secondary; his enemy is latency and routing complexity. If an emergency braking event occurs, the Central Compute must process LiDAR data and issue commands to the brake actuators across heterogeneous networks within milliseconds. 

Marcus expresses this entire nervous system within the `topology.links` section of the World YAML. He defines Node 0 as the Central Compute, Nodes 1-4 as Zonal Controllers, and Nodes 5-20 as edge sensors and actuators. He declares `WireLink` entries of `type: eth` connecting the core, and `type: canfd` for the edge. 

By defining this in the World Schema, Marcus invokes the Parallel Discrete Event Simulation (PDES) barrier. VirtMCU's `DeterministicCoordinator` reads his YAML and enforces strict causal ordering. If a bug causes a race condition between an Ethernet SOME/IP service discovery packet and a CAN-FD diagnostic frame, Marcus can reproduce it perfectly in his CI/CD pipeline. The schema gives Marcus a contractual guarantee: no undocumented network "scouting" can occur. If a link isn't in his YAML, the packets are dropped, perfectly isolating his Zonal boundaries and proving the security of his SDV architecture to regulatory bodies.

---

## See Also
*   **[ADR-010: Platform Description](./adr/ADR-010-platform-description-format.md)**: The rationale behind the move to YAML.
*   **[Determinism and Chaos](./09-determinism-and-chaos.md)**: How `global_seed` and topology influence simulation stability.
*   **[Lesson 3: Parsing .repl to DTB](../tutorials/lesson3-repl2qemu/README.md)**: Hands-on usage of `yaml2qemu.py`.
