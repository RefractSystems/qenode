# VirtMCU Test Guidelines

This document establishes the standards, libraries, and patterns for writing multi-node, deterministic integration tests for the VirtMCU framework.

## 1. Core Architecture & FlatBuffers (The `vproto` layer)
VirtMCU uses **FlatBuffers** as the definitive Interface Definition Language (IDL) for all simulation-layer communication (e.g., `ZenohFrameHeader`, `ClockAdvanceReq`, `MmioReq`). 

### 🚫 Anti-Pattern (What MUST NOT be done)
You must **never** use Python's `struct.pack()` or `struct.unpack()` to manipulate simulation packets manually. You must also **never** hardcode packet slicing boundaries like `raw[20:]` or `raw[:24]`.

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
from tests.conftest import VirtualTimeAuthority

@pytest.mark.asyncio
async def test_my_feature(zenoh_router, zenoh_coordinator, qemu_launcher, zenoh_session, tmp_path):
    # 1. Setup topics and args
    topic_rx = "virtmcu/my_feature/0/rx"
    
    # 2. Launch QEMU Nodes (using fixtures)
    bridge0 = await qemu_launcher(
        node_id=0,
        extra_args=["-device", f"my_feature,topic={topic_rx}"]
    )
    await bridge0.start_emulation()

    # 3. Establish Virtual Time Authority (Deterministic Clock)
    vta = VirtualTimeAuthority(zenoh_session, node_ids=[0])

    # 4. Inject network data securely
    vtime = vta.current_vtimes[0]
    payload = b"TEST"
    header = vproto.ZenohFrameHeader(vtime + 1_000_000, 0, len(payload)).pack()
    
    zenoh_session.put(topic_rx, header + payload)

    # 5. Advance Simulation Time (Never use time.sleep for logic!)
    await vta.step(10_000_000)

    # 6. Verify Results
    assert True # Add your verifications here
