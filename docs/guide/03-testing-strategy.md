# Chapter 3: Testing Strategy & Guidelines

## Quality at Scale

To maintain "Binary Fidelity" and global determinism, VirtMCU employs a multi-layered testing strategy. We prioritize automated, deterministic verification over manual inspection at every stage of the development lifecycle.

---

## 1. The Testing Pyramid

### Tier 1: Unit Tests (Fast & Logic-Only)
*   **Rust**: `cargo test` within each peripheral crate. Focuses on register state machines and IRQ logic without QEMU.
*   **Python**: `pytest` for `yaml2qemu`, `vproto`, and `mcp_server` logic.

### Tier 2: Integration Tests (QEMU + Plugins)
*   Executes a single QEMU node with a minimal guest payload (usually a "smoke app") to verify MMIO routing, clock synchronization, and peripheral registration.

### Tier 3: Multi-Node Stress Tests
*   Orchestrates multiple QEMU nodes, Zenoh routers, and a `TimeAuthority`. Verifies causal ordering, synchronization barrier stability, and network throughput under heavy host load.

---

## 2. Safe Serialization: The `vproto` Layer

VirtMCU uses FlatBuffers for all simulation-layer communication. Developers must **never** manipulate simulation packets using manual byte slicing or Python's `struct` module.

### The `vproto` Standard
Always import the `vproto` wrapper and use the generated classes:
```python
import vproto

# ✅ CORRECT: Schema-safe encoding
payload = vproto.ClockAdvanceReq(delta, vtime, quantum).pack()

# ✅ CORRECT: Schema-safe decoding
header = vproto.ZenohFrameHeader.unpack(data[:vproto.SIZE_ZENOH_FRAME_HEADER])
```
This ensures that any change to the `core.fbs` schema is automatically propagated to all tests, preventing silent protocol desyncs.

---

## 3. Deterministic Testing: The "No-Sleep" Policy

To ensure tests are 100% reproducible and immune to CI load (e.g., under ASan), **wall-clock sleeping is strictly banned.**

### 🚫 Banned: `asyncio.sleep` and `time.sleep`
Using `sleep` to wait for I/O or process initialization is non-deterministic. It will eventually flake.

### 🚫 Banned: Raw `zenoh.open()` in Parallel Tests
By default, Zenoh opens in peer mode with multicast scouting enabled. In parallel `pytest` runs, workers will silently discover each other across the network namespace and cross-talk on shared topics. This is the #1 cause of "passes locally, fails in CI" races.

### ✅ Mandated: Client-Mode Isolation & Synchronization
All Zenoh sessions MUST be opened in client mode with scouting disabled.
1. **Isolation**: Use `make_client_config(connect=router_url)` to build a safe config.
2. **Automated Synchronization (SOTA)**: The framework owns the entire freeze/cont lifecycle. It implicitly injects the `-S` flag to launch QEMU in a frozen state, synchronizes Zenoh routing (`ensure_session_routing`), and issues the final `cont` command. Tests MUST NOT pass `-S` manually, call `ensure_session_routing` directly, or manually instantiate core orchestration components like `QmpBridge` or `VirtualTimeAuthority`.

```python
# ✅ CORRECT: Wakes instantly via signal, respects virtual time limits
await bridge.wait_for_line_on_uart("INIT_DONE", timeout=10.0)

# ✅ CORRECT: Advances the simulation clock strictly
await sim_transport.step_clock(10_000_000)
```

---

## 4. Timeout Scaling

VirtMCU tests are "ASan-Aware." When running under AddressSanitizer, the host CPU can be 5–10x slower. The test harness automatically scales logical timeouts via `get_time_multiplier()`. Developers should always write timeouts based on "real-time" expectations; the infrastructure handles the scaling.

## 5. Local Stress Testing

When developing new features or debugging flaky tests, you must prove stability by running the test repeatedly under load. We provide a utility script to automate this:

```bash
# Run a specific test 20 times (default)
./tools/testing/run_stress.sh tests/test_flexray.py::test_flexray_stress

# Run a test suite 50 times
./tools/testing/run_stress.sh tests/test_spi_stress.py 50
```

## 6. The Single Simulation Entry Point (`Simulation`)

Historically, the project maintained overlapping entry points (`qemu_launcher`, etc.). These have been consolidated into a single SOTA `Simulation` class, exposed via the `simulation` pytest fixture. This consolidation ensures a single, robust lifecycle that is immune to ordering bugs (such as starting emulation before the clock is initialized).

### The Canonical Lifecycle
The framework strictly enforces the following sequence:
1. **Spawn**: All QEMU nodes are launched frozen (`-S` is injected by the framework).
2. **Barrier**: Wait for plugin liveliness barriers across all nodes.
3. **Route**: `ensure_session_routing(session)` is called (framework owned).
4. **Init**: `vta.init()` executes a 0-ns sync while nodes are still frozen.
5. **Start**: QMP `cont` (start_emulation) is issued to all nodes simultaneously.
6. **Teardown**: Strict reverse-order teardown on exit.

### Usage Patterns

#### Single-Node Simulation
```python
async def test_peripheral(simulation):
    # Simple single-node boot
    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel)
    async with simulation as sim:
        await sim.vta.step(1_000_000)
```

#### Multi-Node Simulation
The `Simulation` object supports dynamic node addition before the lifecycle begins. Node IDs are integers (matching the existing `virtmcu-clock,node=N` convention).
```python
async def test_network(simulation):
    simulation.add_node(node_id=0, dtb=dtb0, kernel=k0)
    simulation.add_node(node_id=1, dtb=dtb1, kernel=k1)
    async with simulation as sim:
        # All nodes spawn frozen, complete the barrier sequence,
        # then `cont` is issued simultaneously to all of them.
        await sim.vta.step(1_000_000)
```

### The `inspection_bridge` Escape Hatch
For tests that only require QOM introspection (e.g., verifying register reset values or object properties) without firmware execution or Zenoh traffic, use the `inspection_bridge` fixture.
- **Rule**: Allowed ONLY if no firmware is executed and no Zenoh traffic is generated.
- Nodes remain frozen for the duration of the test; `cont` is never issued.

### Banned Patterns in Tests
To maintain simulation integrity and prevent flaky CI runs, the following patterns are strictly banned and will be enforced by CI lints:
- **Manual `ensure_session_routing(...)`**: The framework handles routing barriers internally.
- **Manual `-S` in `extra_args`**: Framework-injected; manual override breaks synchronization logic.
- **Direct `qemu_launcher` for Firmware**: Use `simulation` for any test that executes guest code.
- **Manual `bridge.start_emulation()`**: Emulation start must be coordinated by the `Simulation` lifecycle.
- **Manual Orchestrator Instantiation**: Do not instantiate orchestrator classes directly in new tests.

---

## 7. Automated Flight Recorder (PCAP)

Debugging complex multi-node failures in CI is challenging. To eliminate the need for parsing thousands of lines of verbose text logs, VirtMCU implements an **Automated Flight Recorder**.

Whenever a `pytest` execution fails (e.g., due to a timeout or failed assertion), the `conftest_core.py` harness automatically dumps the entire test's network traffic history into two artifact formats:
1.  **JSON Trace**: A human-readable list of events containing `vtime_ns`, `topic`, and the hex `payload`.
2.  **PCAP File**: A binary capture file natively readable by Wireshark.

### Locating Artifacts
Artifacts are automatically saved to the `test-results/flight_recorder/` directory:
```
test-results/flight_recorder/test_name.json
test-results/flight_recorder/test_name.pcap
```

### Wireshark Introspection
By opening the `.pcap` artifact in Wireshark, you can observe the exact inter-node traffic, perfectly aligned by their **virtual timestamps**, providing a granular view of exactly what caused a multi-node deadlock or failure. The PCAP uses DLT_USER0 (147) and encapsulates Python-side metrics (topics, direction) directly into the Wireshark-readable payloads via Protocol 255 (sim-tracing).

---

## 8. When to write a Lint, a Test, or a Postmortem

The "FlexRay Incident" taught us that high-quality engineering requires choosing the right tool for the right defect.

| Artifact | When to use it | Goal |
| :--- | :--- | :--- |
| **Lint** | When a bug is caused by a **static disagreement** between files (e.g., name mismatch, layout drift). | Fail at **compile/lint time**. |
| **Unit Test** | When a bug is in **internal logic** (e.g., a state machine transition). | Fail during `cargo test`. |
| **Integration Test** | When a bug is in the **interaction** between components (e.g., QEMU ↔ Zenoh). | Fail during `pytest`. |
| **Postmortem** | When a bug is **complex, cascading, or structural**. | Documentation for **future engineers**. |

### The "Fail Loudly" Principle
If a bug can be caught at lint time, **write a linter**. Do not rely on a runtime test to catch a name mismatch that will only surface as a SIGSEGV in a different part of the system.
