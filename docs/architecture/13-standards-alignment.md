# Standards Alignment: VirtMCU and Established Simulation Frameworks

## Learning Objectives
After this chapter, you can:
1. Map every VirtMCU architectural concept to at least one published standard.
2. Explain why VirtMCU's quantum barrier is a correct CMB conservative PDES implementation.
3. Distinguish the HLA, FMI, and OpenUSD views of the same running simulation.

---

## Overview

VirtMCU is not a novel invention. Its architecture is a disciplined composition of
concepts drawn from decades of distributed simulation research and production engineering
practice. This chapter maps each VirtMCU component and design decision to the established
standard it implements. Where VirtMCU diverges from a standard, the divergence is
intentional and documented.

---

## 1. Parallel Discrete Event Simulation (PDES) — Chandy-Misra-Bryant

**Foundational papers**: Chandy & Misra (1979), Bryant (1979).  
**Textbook coverage**: Fujimoto, *Parallel and Distributed Simulation Systems*, Wiley, 2000.

VirtMCU's quantum barrier is a direct implementation of the Chandy-Misra-Bryant (CMB)
**conservative synchronization** algorithm. Conservative means no node is permitted to
advance its virtual clock unless it is mathematically guaranteed that no message from the
past can arrive.

| CMB Concept | VirtMCU Equivalent |
|---|---|
| Logical Process (LP) | QEMU cyber node |
| Null Message | `ClockReadyResp` — signals "I have no message before T+delta" |
| Conservative barrier | `DeterministicCoordinator` per-quantum barrier |
| Lookahead value | Quantum size (`delta_ns`) |
| Simultaneous-event tie-breaking | Canonical `(delivery_vtime_ns, node_id, seq)` ordering |

The `DeterministicCoordinator` accumulates `ClockReadyResp` signals from every node. Once
all nodes have signalled "quantum Q complete", it releases all buffered messages for Q and
grants advancement to Q+1. This is the CMB barrier condition: *no node will produce a
message timestamped ≤ Q's upper bound*.

**Key difference from textbook CMB**: CMB operates event-by-event. VirtMCU coalesces all
events within a quantum into a single batch. This trades lookahead granularity for reduced
IPC overhead and is the same trade-off used by the HLA Time Management Service (see §3).

For a detailed treatment, see **[PDES and Virtual Time](../fundamentals/08-pdes-and-virtual-time.md)**.

---

## 2. High-Level Architecture (HLA) — IEEE Std 1516-2010

**Standard**: IEEE Standard for Modeling and Simulation High Level Architecture,
IEEE Std 1516-2010.  
**Usage context**: NATO, US DOD, aerospace, automotive multi-body simulation.

HLA is the dominant standard for federating independent simulations. Its vocabulary maps
directly onto VirtMCU:

| HLA IEEE 1516 Term | VirtMCU Equivalent | Notes |
|---|---|---|
| **Federation** | A running simulation instance | Live execution of a declared `World` |
| **Federation Object Model** | World YAML | Declares participants, data types, publish/subscribe rules |
| **Runtime Infrastructure (RTI)** | `DeterministicCoordinator` | Manages time, routing, and membership |
| **Federate** | QEMU node, Physics Gateway, Time Authority | A participant in a running federation |
| **`TimeAdvanceRequest`** | `ClockAdvanceReq` | Federate requests to advance to time T |
| **`TimeAdvanceGrant`** | `ClockReadyResp` | RTI grants advancement — no earlier messages pending |
| **Federate Execution Data** | `topology.nodes[]` in World YAML | Static participant graph |
| **Ownership Management** | Actuator/sensor assignment in World YAML | Which federate publishes which attribute |

### HLA Time Management Alignment

HLA Time Management offers two modes — both are supported by VirtMCU:

- **Time-Stepped** (`TimeAdvanceRequest` with fixed `h`): analogous to
  `slaved-suspend` with fixed `delta_ns`.
- **Event-Driven** (`NextEventRequest`): analogous to `slaved-icount` where QEMU
  advances to the next scheduled peripheral event rather than a fixed boundary.

In both cases, the RTI (our `DeterministicCoordinator`) issues a `TimeAdvanceGrant` only
when no federate can produce a message with timestamp ≤ the requested time. This is
mathematically identical to VirtMCU's quantum release condition.

### Terminology Adopted from HLA

VirtMCU adopts **Federation** as the runtime concept to distinguish the *running instance*
from the *World* (the static YAML declaration):

```
World (YAML manifest)  ─── instantiates ──►  Federation (running instance)
                                              ├── QEMU Node 0  (Federate)
                                              ├── QEMU Node 1  (Federate)
                                              ├── Physics Gateway  (Federate)
                                              └── DeterministicCoordinator  (RTI)
```

A `--federation-id` CLI flag identifies running instances when multiple federations share
the same transport bus. The World YAML is the Federation Object Model: it remains unchanged on disk regardless
of how many times it is instantiated.

---

## 3. Functional Mock-up Interface (FMI) — FMI 3.0

**Standard**: FMI 3.0 for Co-Simulation, Modelica Association, 2023.  
**URL**: https://fmi-standard.org/  
**Usage context**: AUTOSAR, MATLAB/Simulink, Modelon Impact, Dymola, OpenModelica.

FMI is the dominant automotive standard for co-simulation. VirtMCU's quantum loop
implements the FMI co-simulation master algorithm:

| FMI 3.0 Concept | VirtMCU Equivalent |
|---|---|
| **Co-simulation master** | Physical Node (`virtmcu-physical-node`) |
| **Co-simulation slave (FMU)** | QEMU node — executes firmware within one time step |
| **Communication step** (`h`) | Quantum size (`delta_ns`) |
| **`doStep(currentTime, h)`** | `ClockAdvanceReq {absolute_vtime_ns, delta_ns}` |
| **`setInputs` before `doStep`** | Sensor publication via Zenoh before next quantum |
| **`getOutputs` after `doStep`** | Actuator drain from `ZenohActuatorSink` after `ClockReadyResp` |
| **`intermediateUpdate` callback** | Not implemented — VirtMCU is batch, not intra-step |

FMI's optional "rollback" capability (`canGetAndSetFMUstate`) is explicitly excluded:
rolling back a running QEMU virtual machine is impractical. VirtMCU is a fixed-step
co-simulation master with no rollback, matching the FMI
`canGetAndSetFMUstate = false` profile.

### Future: FMU Wrapping

Because `ClockAdvanceReq` / `ClockReadyResp` matches FMI's `doStep` semantics exactly,
a future `FmuAdapter` could wrap a VirtMCU cyber node as a standard FMU. This would
allow firmware-in-the-loop (FIL) validation inside MATLAB/Simulink or AUTOSAR tool
chains without modifying the firmware binary.

---

## 4. OpenUSD — Universal Scene Description

**Standard**: Pixar OpenUSD (open source, `openusd.org`), NVIDIA Omniverse Kit.  
**Alignment**: [RFC-0010](../rfcs/0010-platform-description-format.md),
[RFC-0016](../rfcs/0016-logical-domain-model.md).

VirtMCU's World YAML is designed as a transitional format toward OpenUSD-native scene
description. The mapping is formalised in RFC-0016:

| OpenUSD Concept | VirtMCU Equivalent |
|---|---|
| `UsdStage` | `World` (top-level YAML document) |
| `UsdGeomXform` | `Node` (execution entity with optional 3D pose) |
| `UsdPrim` | Any `Machine`, `Resource`, or `Link` |
| `UsdAttribute` | Peripheral property (`baudrate`, `base_address`, etc.) |
| `UsdRelationship` | `Link` or `interrupt` connection |
| `CyberPrim` (custom schema) | QEMU node as first-class scene participant |

When a VirtMCU cyber node is deployed inside NVIDIA Omniverse, it becomes a `CyberPrim`
inside the physics scene. The Physics Gateway bridges the Omniverse physics solver
(USD native) to the QEMU emulated MCU over the SHM protocol — the robot geometry and
the virtual firmware share one `UsdStage`.

### Stage ≠ Federation

In OpenUSD, a "Stage" is the loaded, in-memory representation of a `.usd` scene.
VirtMCU uses a three-tier vocabulary:

```
World  (YAML manifest, the Federation Object Model)
  └─── instantiated at runtime as ────►  Federation  (running instance)
                                             └─── future USD path ──►  UsdStage
```

The USD `Stage` is a future persistence path for the `World` manifest, not a synonym for
`Federation`. This distinction matters for RFC-0016 implementation work.

---

## 5. OMG DDS — Data Distribution Service

**Standard**: OMG Data Distribution Service for Real-Time Systems, v1.4.  
**Relevance**: Zenoh's pub/sub model follows the DDS conceptual model.

| DDS Concept | Zenoh / VirtMCU Equivalent |
|---|---|
| **Domain** | Zenoh session (explicit endpoint list; multicast banned in production) |
| **Topic** | Zenoh key expression (`firmware/control/0/7`) |
| **DataWriter** | Zenoh `Publisher` |
| **DataReader** | Zenoh `Subscriber` |
| **QoS: Reliability** | `Reliability::Reliable` on the clock control plane |
| **QoS: Durability** | Not used — VirtMCU messages are ephemeral |
| **QoS: Deadline** | Enforced externally by quantum timeout in Time Authority |

Zenoh is not a DDS implementation, but it is the semantic successor for edge robotics
(ROS2 supports Zenoh via `rmw_zenoh_cpp`). VirtMCU's message bus was designed with this
lineage in mind — see [RFC-0011](../rfcs/0011-zenoh-federation-bus.md).

---

## 6. IEEE 1666 SystemC / OSCI TLM-2.0

**Standard**: IEEE 1666-2011 (SystemC), OSCI TLM-2.0.  
**Relevance**: VirtMCU's hardware co-simulation path (Remote Port).

When VirtMCU connects to a Verilator RTL model or a SystemC design under test:

| TLM-2.0 Concept | VirtMCU Equivalent |
|---|---|
| `b_transport` initiator | QEMU memory bus (initiates MMIO accesses) |
| `b_transport` target | Remote Port Slave (receives TLM-2.0 payloads over IPC) |
| `tlm_generic_payload` | MMIO read/write struct (address, data, byte enables) |
| `sc_time` | Virtual time in nanoseconds (`delivery_vtime_ns`) |
| `sc_module` | QOM device plugin (`.so`) |

This path supports hardware-in-the-loop (HIL) scenarios where the peripheral under test
is a synthesisable RTL description rather than a QEMU device model.

---

## 7. SysML v2 — Systems Modeling Language

**Standard**: OMG SysML v2 (2024), Object-Oriented Systems Engineering Method (OOSEM).  
**Alignment**: [RFC-0016](../rfcs/0016-logical-domain-model.md).

VirtMCU's World YAML maps mechanically to a SysML v2 Block Definition Diagram (BDD):

| SysML v2 Concept | VirtMCU Equivalent |
|---|---|
| `block` | `Node` or `Machine` |
| `part property` | Peripheral (memory-mapped resource) |
| `connector` | `Link` (protocol-typed connection) |
| `value property` | Peripheral attribute (`base_address`, `baudrate`) |
| `constraint block` | Semantic invariant ("no link without declared node") |
| `activity diagram` | Quantum loop (`ClockAdvanceReq` → all nodes respond → `ClockReadyResp`) |

A World YAML can be authored by systems engineers in a SysML-2 tool and round-tripped to
YAML without information loss, supporting formal MBSE workflows for safety-critical
firmware validation.

---

## 8. Summary Alignment Table

| VirtMCU Concept | HLA IEEE 1516 | FMI 3.0 | OpenUSD | PDES / CMB | DDS |
|---|---|---|---|---|---|
| `World` (YAML) | Federation Object Model | FMU description XML | `UsdStage` | Simulation spec | Not applicable |
| `Federation` (runtime) | Federation | Co-simulation session | Loaded Stage | Running simulation | Domain |
| `DeterministicCoordinator` | RTI | Co-sim master (partial) | — | PDES barrier | — |
| QEMU cyber node | Federate | FMU slave | `CyberPrim` | Logical Process | DataReader/Writer |
| Physics Gateway | Federate | FMU slave | `PhysicsPrim` | Logical Process | DataReader/Writer |
| Quantum (`delta_ns`) | HLA timestep | Communication step `h` | — | Lookahead window | — |
| `ClockAdvanceReq` | `TimeAdvanceRequest` | `doStep(t, h)` | — | Null-message / advance | — |
| `ClockReadyResp` | `TimeAdvanceGrant` | `doStep` return | — | Ready signal | — |
| `PhysicsTrigger` | Interaction class update | `setInput` + `doStep` | — | Cross-LP event batch | Publication |
| Zenoh key expression | Interaction class name | Variable reference | `UsdAttribute` path | Channel identifier | Topic |
| `global_seed` | Not in HLA | Not in FMI | — | Stochastic extension | Not applicable |

---

## 9. What VirtMCU Adds Beyond the Standards

Standards define the *what*; VirtMCU specifies the *how* for firmware emulation:

1. **Binary Fidelity constraint** — not present in HLA, FMI, or PDES literature. An FMU
   or HLA federate may use any internal model. VirtMCU mandates the exact same ELF binary
   that runs on real silicon, with bit-accurate peripheral registers.

2. **Deterministic stochastic seeding** (`seed_for_quantum(global_seed, node_id, Q)`) —
   HLA and FMI are silent on PRNG discipline across federates. VirtMCU bans `rand::thread_rng()`.

3. **PDES + Physics co-simulation in one barrier** — standard PDES literature handles
   LP-to-LP messages. VirtMCU extends the barrier to include a physics engine step
   (`PhysicsTrigger` / `PhysicsDone`) before granting the next quantum, which is not
   described in any published standard.

4. **SHM futex protocol for physics engine integration** — no standard prescribes the
   physical memory interface between the co-simulation master and a physics engine.
   VirtMCU's `/dev/shm/virtmcu_physics_{node_id}` layout (§3 of
   [Physics Gateway](./12-physics-gateway.md)) is a VirtMCU-specific protocol.

---

## See Also

- **[PDES and Virtual Time](../fundamentals/08-pdes-and-virtual-time.md)**: CMB algorithm
  in depth.
- **[The Temporal Core](./02-temporal-core.md)**: `ClockAdvanceReq`/`ClockReadyResp`
  implementation.
- **[Physics Gateway](./12-physics-gateway.md)**: FMI-style master algorithm extended
  with a physics step.
- **[RFC-0016: Logical Domain Model](../rfcs/0016-logical-domain-model.md)**: OpenUSD and
  SysML mapping formalised.
- **[RFC-0011: Zenoh Federation Bus](../rfcs/0011-zenoh-federation-bus.md)**: Why Zenoh
  over standard DDS.
- **[World Specification](./10-world-specification.md)**: The YAML schema (the Federation Object Model).

