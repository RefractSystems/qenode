# VirtMCU Test Guidelines

This document establishes the standards, libraries, and patterns for writing multi-node, deterministic integration tests for the VirtMCU framework.

## 1. Core Architecture & FlatBuffers (The `vproto` layer)
VirtMCU uses **FlatBuffers** as the definitive Interface Definition Language (IDL) for all simulation-layer communication (e.g., `ZenohFrameHeader`, `ClockAdvanceReq`, `MmioReq`). 

### 🚫 Anti-Pattern (What MUST NOT be done)
You must **never** use Python's `struct.pack()`, `struct.unpack()`, or `struct.unpack_from()` to manipulate simulation packets manually. You must also **never** hardcode packet slicing boundaries like `raw[20:]` or `raw[:24]`.

```python
# 🚫 BAD: Hardcoded struct.pack (will fail CI linting)
payload = struct.pack("<QQQ", delta, vtime, quantum)

# 🚫 BAD: Hardcoded magic sizes
header = data[:24]
payload = data[24:]
```

### ✅ Standard Pattern (What MUST be done)
Always import `vproto` and use the auto-generated FlatBuffers classes and size constants.

```python
# ✅ GOOD: Safe, auto-generated IDL
import vproto

# Encoding
payload = vproto.ClockAdvanceReq(delta, vtime, quantum).pack()

# Decoding & Slicing
header = vproto.ZenohFrameHeader.unpack(data[:vproto.SIZE_ZENOH_FRAME_HEADER])
payload = data[vproto.SIZE_ZENOH_FRAME_HEADER:]
```

## 2. Infrastructure & Lifecycle Management
Multi-node testing requires orchestrating Zenoh routers, python bridges, QEMU nodes, and deterministic clocks.

- Always use the `AsyncManagedProcess` context manager from `tools.testing.virtmcu_test_suite.process` for background tasks to ensure no orphaned processes remain.
- Tests should be marked with `@pytest.mark.asyncio`.
- Rely on standard fixtures from `tests/conftest.py` (`zenoh_router`, `zenoh_session`, `qemu_launcher`, `zenoh_coordinator`).

## 3. Generic Multi-Node Test Template

```python
import asyncio
import pytest
import vproto

@pytest.mark.asyncio
async def test_my_feature(qemu_launcher, sim_transport, tmp_path):
    # 1. Setup topics and args
    topic_rx = "virtmcu/my_feature/0/rx"
    
    # 2. Launch QEMU Nodes (using fixtures)
    bridge0 = await qemu_launcher(
        dtb_path, kernel_path, # Provide paths to your compiled artifacts
        extra_args=[
            "-device", sim_transport.get_clock_device_str(node_id=0),
            "-device", f"my_feature,node=0,{sim_transport.get_peripheral_props()},topic={topic_rx}"
        ],
        ignore_clock_check=True
    )
    await bridge0.start_emulation()

    # 3. Inject network data securely (using SimulationTransport)
    await sim_transport.step_clock(0) # Initial clock sync
    
    payload = b"TEST"
    # Provide the properly padded IDL wrapper around your payload, e.g. LIN Frame or SPI Frame.
    # Note: SimulationTransport handles the Zenoh / Unix wrapping!
    
    await sim_transport.publish(topic_rx, payload)

    # 4. Advance Simulation Time (Never use time.sleep for logic!)
    await sim_transport.step_clock(10_000_000)

    # 5. Verify Results
    assert True # Add your verifications here
```


## 4. Determinism & Banned Practices: Sleep and Polling (ARCH-20)

To guarantee that tests are 100% deterministic and do not flake under heavy CI load (like ASan), **wall-clock sleeping is strictly banned**.

### 🚫 Anti-Pattern: `asyncio.sleep` and `time.sleep`
You must **never** use `await asyncio.sleep(0.1)` to wait for a network message to arrive or for a process to initialize.
```python
# 🚫 BAD: Will fail CI linting
for _ in range(100):
    if "INIT_DONE" in bridge.uart_buffer:
        break
    await asyncio.sleep(0.1)
```

### ✅ Standard Pattern: Event Signaling and Virtual Time
Use the explicit event-driven helpers provided by the `QmpBridge` and `AsyncManagedProcess`:
```python
# ✅ GOOD: Wakes instantly via asyncio.Event, respects virtual time limits
await bridge.wait_for_line_on_uart("INIT_DONE", timeout=10.0)

# ✅ GOOD: Advances the determinism clock strictly
await vta.step(10_000_000)
```

If you are writing a loop that polls a buffer or needs to wait for background I/O (like Zenoh subscribers, QMP readers, or process pipes) to catch up, you MUST use `yield_now()` to explicitly relinquish control to the event loop.

```python
from tools.testing.utils import yield_now

# ✅ GOOD: Enterprise-grade yield for asyncio
while not condition_met():
    await yield_now()
```

If you are writing a mock hardware node that explicitly needs to simulate physical execution delay to prevent Zenoh publisher race conditions, you may use sleep but **must** annotate it:
```python
import time
time.sleep(0.1) # SLEEP_EXCEPTION: mock node simulating execution time to avoid Zenoh publisher race condition
```

## 5. Execution Pacing & Timeout Scaling (INFRA-6 & INFRA-9)

Tests must be robust against Host CPU slowdowns without burdening the developer with mental math.

- **Logical Timeouts**: Always write timeouts based on ideal conditions (e.g., `timeout=10.0`). The `SimulationTransport` and `QmpBridge` automatically scale this via `get_time_multiplier()` (e.g., 5x for ASan).
- **Faster-Than-Real-Time (FTRT)**: By default, CI runs with a pacing multiplier of `0.0`, meaning simulation virtual time advances as quickly as the host CPU computes. 
