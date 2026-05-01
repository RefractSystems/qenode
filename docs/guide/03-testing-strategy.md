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
*   Orchestrates multiple QEMU nodes, Zenoh routers, and a `TimeAuthority`. Verifies causal ordering, ARCH-8 barrier stability, and network throughput under heavy host load.

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

### ✅ Mandated: Event Signaling & Virtual Time
Use the event-driven helpers provided by the `QmpBridge` and `SimulationTransport`:
```python
# ✅ CORRECT: Wakes instantly via signal, respects virtual time limits
await bridge.wait_for_line_on_uart("INIT_DONE", timeout=10.0)

# ✅ CORRECT: Advances the simulation clock strictly
await sim_transport.step_clock(10_000_000)
```

---

## 4. Timeout Scaling (INFRA-6)

VirtMCU tests are "ASan-Aware." When running under AddressSanitizer, the host CPU can be 5–10x slower. The test harness automatically scales logical timeouts via `get_time_multiplier()`. Developers should always write timeouts based on "real-time" expectations; the infrastructure handles the scaling.

## 5. Local Stress Testing

When developing new features or debugging flaky tests, you must prove stability by running the test repeatedly under load. We provide a utility script to automate this:

```bash
# Run a specific test 20 times (default)
./tools/testing/run_stress.sh tests/test_flexray.py::test_flexray_stress

# Run a test suite 50 times
./tools/testing/run_stress.sh tests/test_spi_stress.py 50
```

## 6. The Declarative Simulation Environment (`VirtmcuSimulation`)

A critical lesson learned during extreme-load ASan/TSan stress testing was the vulnerability of manual test orchestration. If developers must manually sequence QEMU boot, Zenoh discovery, and clock initialization, simple omissions (like forgetting a `wait_for_discovery()` barrier) inevitably lead to race conditions, dropped packets, and endless polling deadlocks.

To structurally prevent this and provide an "idiot-proof" API, VirtMCU uses the `simulation` fixture to expose a declarative `VirtmcuSimulation` orchestrator.

### The Orchestration Lifecycle

You no longer need to instantiate `QmpBridge` or `VirtualTimeAuthority` manually. Instead, use the declarative context manager:

```python
@pytest.mark.asyncio
async def test_my_feature(simulation):
    dtb, kernel = build_artifacts()
    extra_args = ["-device", "virtmcu-clock,node=0,mode=slaved-icount"]

    # 1. Bring-Up & Setup
    async with await simulation(dtb, kernel, extra_args=extra_args) as sim:
        
        # 2. Test Logic (System is guaranteed to be perfectly frozen at 0 ns)
        await sim.vta.step(1_000_000)
        
    # 3. Teardown (Automatically handles QEMU termination & Zenoh cleanup)
```

The `VirtmcuSimulation` orchestrator handles three distinct stages behind the scenes:

1. **Bring-Up (Deterministic Setup):** 
   - Starts the QEMU instance and ensures the `-S` (freeze at boot) flag is present.
   - Automatically injects `router={zenoh_router}` into your `-device` and `-chardev` CLI arguments so you don't have to manage network topologies manually.
   - Waits for the QMP and UART sockets to appear and connects the bridge.
2. **The Deterministic Initialization Barrier (`sim.vta.init()`):**
   - Automatically waits for the `sim/clock/liveliness/{nid}` tokens for all managed nodes (Global Liveliness Barrier).
   - Implicitly executes a `0 ns` clock advance to ensure QEMU is fully booted, connected, and frozen *exactly* at `vtime = 0 ns` before yielding control to your test logic.
3. **Teardown:**
   - Safely closes the QMP bridge and guarantees QEMU processes are reaped, even if your test assertions fail.

By encapsulating discovery and initialization into this declarative context manager, tests are significantly shorter, cleaner, and completely immune to race conditions!

## 7. Automated Flight Recorder (PCAP)

Debugging complex multi-node failures in CI is challenging. To eliminate the need for parsing thousands of lines of verbose text logs, VirtMCU implements an **Automated Flight Recorder** (INFRA-7).

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
