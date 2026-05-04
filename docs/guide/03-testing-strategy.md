# Chapter 3: Testing Strategy & Guidelines

## Quality at Scale

To maintain "Binary Fidelity" and global determinism, VirtMCU employs a multi-layered testing strategy. We prioritize automated, deterministic verification over manual inspection at every stage of the development lifecycle.


---

## 4. Test Directory Architecture & The Transport Agnosticism Mandate

Our test suite is organized strictly by **System-Under-Test (SUT)** to maintain clear separation of concerns.

### Directory Structure

```text
tests/
├── unit/                  # Fast, isolated white-box logic. No QEMU allowed.
├── fixtures/              # Topologies and minimal guest applications.
├── firmware/              # Golden SDK binaries for compatibility regression.
└── integration/           # Python-orchestrated QEMU lifecycle tests.
    ├── simulation/        # SUT: Guest Firmware & QEMU Peripherals.
    ├── infrastructure/    # SUT: The VirtMCU Framework & Transport.
    └── tooling/           # SUT: Out-of-band orchestration tools.
```

### The Transport Agnosticism Mandate
Tests located in `tests/integration/simulation/` test the **firmware** and **peripherals**. They must treat the underlying network as a "dumb pipe". 
* **BANNED**: Directly importing or orchestrating Zenoh (`import zenoh`, `zenoh_session`).
* **REQUIRED**: All tests must use the `SimulationTransport` abstraction (`sim.transport.publish()`).

Tests in `tests/integration/infrastructure/` explicitly test the Zenoh routing or PDES barrier mechanisms. They are permitted to bypass the transport layer, but **must** declare their intent at the top of the file using the `# ZENOH_HACK_EXCEPTION: <reason>` annotation.

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

---

## 9. Infrastructure Orchestration: The Golden Template

When testing VirtMCU's out-of-band infrastructure components (like `deterministic_coordinator` or `zenoh_router`), we must strictly avoid "shadow orchestration" (manually spinning up threads or using banned subprocess APIs).

To maintain SOTA (State of the Art) test stability, deterministic timing, and interleaved logging, all infrastructure tests **must** use the `ManagedSubprocess` + `asyncio.Queue` pattern.

### Why this is the "Golden Template"
1.  **Unified Logging**: `ManagedSubprocess` automatically captures `stdout`/`stderr` and streams them via `logger.info()`. If a test fails, the background process's logs are already perfectly interleaved with `pytest` and QEMU output.
2.  **Thread-Safe Zenoh Interop**: Zenoh callbacks fire on background threads. Directly interacting with `pytest` state from these threads causes flaky race conditions. Using `asyncio.get_running_loop().call_soon_threadsafe()` to push to an `asyncio.Queue` perfectly bridges the gap back to the test's main async loop.
3.  **Simulation Hygiene Compliance**: It passes the strict AST lints (`TID251`) that ban manual `subprocess.run` calls in test bodies.

### The Template Code
This is the mandated pattern for any test that needs to spin up a background tool and listen to its Zenoh output:

```python
import asyncio
import logging
import pytest
import zenoh
from tools.testing.virtmcu_test_suite.conftest_core import ManagedSubprocess, get_free_endpoint

logger = logging.getLogger(__name__)

class InfrastructureTester:
    """Helper to manage Zenoh subscribers and safely route them back to the async loop."""
    def __init__(self, session: zenoh.Session):
        self.session = session
        self.rx_queues: dict[str, asyncio.Queue[bytes]] = {}
        self.subscribers: list[zenoh.Subscriber] = []
        # MUST capture the loop in the main thread
        self.loop = asyncio.get_running_loop()

    def setup_subscriber(self, topic: str) -> asyncio.Queue[bytes]:
        q: asyncio.Queue[bytes] = asyncio.Queue()
        self.rx_queues[topic] = q

        def _on_sample(sample: zenoh.Sample) -> None:
            # Safely bounce the Zenoh background thread callback to the async loop
            self.loop.call_soon_threadsafe(q.put_nowait, sample.payload.to_bytes())

        sub = self.session.declare_subscriber(topic, _on_sample)
        self.subscribers.append(sub)
        return q

    async def wait_for_frame(self, topic: str, timeout: float = 5.0) -> bytes:
        from tools.testing.utils import get_time_multiplier
        q = self.rx_queues[topic]
        # Automatically scales timeout for slow CI environments (e.g. ASan)
        return await asyncio.wait_for(q.get(), timeout=timeout * get_time_multiplier())

    def close(self) -> None:
        for sub in self.subscribers:
            sub.undeclare()

@pytest.mark.asyncio
async def test_my_infrastructure_tool(zenoh_session: zenoh.Session) -> None:
    endpoint = get_free_endpoint()
    cmd = ["python3", "-m", "tools.my_tool", endpoint]
    
    # 1. Use ManagedSubprocess for interleaved logging and automatic cleanup
    async with ManagedSubprocess("my_tool", cmd) as _proc:
        tester = InfrastructureTester(zenoh_session)
        try:
            # 2. Setup safe subscribers
            tester.setup_subscriber("sim/my_tool/out")
            
            # 3. Test logic...
            data = await tester.wait_for_frame("sim/my_tool/out")
            assert b"expected" in data
            
        finally:
            tester.close()
```
