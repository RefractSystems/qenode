# RFC-0001: Project Vision, Target Audience, and Core Constraints

## Summary
This RFC formalizes the overarching project vision, defines the exact target customer for VirtMCU 1.0, and establishes the immutable technical constraints that all future proposals must satisfy. It acts as the "Constitution" for evaluating whether an architectural change aligns with the framework's primary goals.

## Motivation
As VirtMCU transitions to an open RFC model and approaches its 1.0 release, community contributions will accelerate. Without a formalized vision, the framework risks architectural drift—for example, accepting features that prioritize raw "video game" throughput over deterministic correctness, or allowing "simulation-only" firmware hacks that break binary fidelity. We must define *who* we are building this for and *what* rules cannot be broken.

## Target Audience (The Customer)

VirtMCU is built for **Enterprise Firmware Engineers and Automated HIL/SIL Pipeline Operators** (e.g., users of Firmware Studio).

These users have a specific profile:
- They develop mission-critical cyber-physical systems (automotive, aerospace, industrial robotics).
- They rely on Continuous Integration (CI) pipelines that require test runs to be reproducible across 1000s of parallel containers.
- They are tired of "sim-only" bugs where firmware works in an emulator but crashes on silicon because of mismatched register addresses or lazy timing models.
- They need to orchestrate complex topologies (multiple MCUs communicating over CAN-FD, FlexRay, or Ethernet) in lock-step.

VirtMCU is **not** targeted at casual hobbyists looking for a quick, approximate emulator, nor is it targeted at cycle-accurate RTL verification engineers who need to analyze gate-level propagation delays.

## The Core Constraints (What & How It MUST Work)

To serve the target audience, any future RFC or architecture change MUST adhere to the following immutable constraints:

### 1. Absolute Binary Fidelity
**What it is:** The exact ELF binary compiled for the physical silicon must run unmodified in VirtMCU.
**The Constraint:**
- No "sim-only" macros, linker scripts, or semi-hosting backdoors that modify how the firmware executes.
- Peripheral MMIO layouts, reset values, and interrupt wirings must perfectly match the vendor datasheet. 
- If a future RFC proposes a "convenience" feature that requires firmware to know it is in a simulator, it will be rejected.

### 2. Global Simulation Determinism
**What it is:** The same inputs (Topology YAML + Firmware ELFs + Global Seed) must produce a bit-for-bit identical output on every single run, regardless of host CPU speed or network jitter.
**The Constraint:**
- All network interactions and sensor data injections must be delivered based on **Virtual Time (vtime)**, never wall-clock time.
- Randomness must be deterministically seeded via `global_seed`. No `rand::thread_rng()` or `/dev/urandom` allowed in peripheral state machines.
- All RFCs introducing new asynchronous behavior must explain how they synchronize with the `DeterministicCoordinator` and the Big QEMU Lock (BQL).

### 3. "Crash-Only" Enterprise SOTA Design (Fail Loudly)
**What it is:** The framework must immediately abort rather than attempt to gracefully recover from an invalid simulation state.
**The Constraint:**
- Warnings for broken internal invariants are banned. If a state is invalid, the simulation must crash (`panic!` or `.expect()`) to prevent silent divergence.
- Resource management must use RAII. 
- Thread deadlocks (especially involving the BQL) must be prevented by design (e.g., using the `Peripheral` trait, `MmioResult::wait_for`, and Yield-on-Read patterns from RFC-0018) rather than "retry" loops.

### 4. Multi-Tier Single Source of Truth (SSOT)
**What it is:** To prevent "Ghost Mismatches" (where firmware, emulators, and UI disagree on hardware layout), no hardware parameter can ever be defined twice. Every data point must have a single authoritative origin and flow unidirectionally into generated artifacts.
**The Constraint:**
- Hardcoding peripheral offsets, network layouts, or payload layouts in application logic (Rust, C/C++, or Python) is strictly banned.
- All definitions must originate from the authoritative tiers and be auto-generated into code.

| Tier | Source of Truth | Scope | Automation Path |
| :--- | :--- | :--- | :--- |
| **Micro-Architecture** | **CMSIS-SVD (`.svd`)** | Register offsets, bitfields, base addresses. | `virtmcu-cli svd2header` & `svd2schema` |
| **Macro-Architecture** | **Topology YAML (`.yaml`)** | Node connectivity, peripheral instantiation. | `yaml2qemu` -> DeviceTree (`.dtb`) |
| **Wire Protocol** | **FlatBuffers (`.fbs`)** | Payload layouts for Zenoh/UDS messages. | `flatcc` / `virtmcu-wire` crate |

**Data Flow Mandate (Where things come from and go):**
1. **SVDs** (from Silicon Vendors) -> Generate `robot_io.h` (for Firmware) and `svd_constants.rs` (for Emulator backend).
2. **Topology YAMLs** (from Integrators) -> Read by Orchestrator to generate `-device` args and QEMU Device Trees (`.dtb`).
3. **FlatBuffers** (from VirtMCU Architects) -> Generate Python/C++ schemas for downstream dashboards and SystemC bridges.

## Drawbacks
By cementing these constraints:
- We explicitly reject high-performance optimizations that rely on non-deterministic event loops (e.g., standard `tokio` multi-threading without vtime barriers).
- Developing new peripherals requires significantly more rigor (understanding BQL yielding, flatbuffers, and vtime synchronization) compared to simpler emulators.

## Rationale and Alternatives
We could operate without a formalized vision and judge PRs on a case-by-case basis. However, this leads to reviewer fatigue and inconsistent architectural decisions. By writing this down as RFC-0001, we provide a concrete rubric. When a community member proposes an idea, we can ask: *"Does this preserve Absolute Binary Fidelity?"* If the answer is no, the proposal can be swiftly and politely closed.

## Unresolved questions
None. This document codifies the existing, implicitly understood goals that have driven development up to this point.