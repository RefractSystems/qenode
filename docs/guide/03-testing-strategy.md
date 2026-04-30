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
./tools/testing/run_stress.sh tests/test_phase20_5_stress.py 50
```

The script will instantly halt and report the failure if any iteration fails, allowing you to inspect the system state and logs at the exact moment of failure.
