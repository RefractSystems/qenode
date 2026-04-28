import struct
from collections import namedtuple
from unittest.mock import AsyncMock

import pytest
import vproto
from conftest import VirtualTimeAuthority


def pack_clock_ready(vtime_ns: int, n_frames: int = 0, error_code: int = 0, quantum_number: int = 0) -> bytes:
    return vproto.ClockReadyResp(vtime_ns, n_frames, error_code, quantum_number).pack()

@pytest.mark.asyncio
async def test_no_overshoot_when_exact():
    vta = VirtualTimeAuthority(session=None, node_ids=[1])

    # Mock _get_reply to return a valid Zenoh reply structure
    Reply = namedtuple("Reply", ["ok"])
    Ok = namedtuple("Ok", ["payload"])
    Payload = namedtuple("Payload", ["to_bytes"])

    mock_reply = Reply(ok=Ok(payload=Payload(to_bytes=lambda: pack_clock_ready(10_000_000))))
    vta._get_reply = AsyncMock(return_value=mock_reply)

    await vta.step(10_000_000)

    assert vta._overshoot_ns[1] == 0
    assert vta.current_vtimes[1] == 10_000_000

@pytest.mark.asyncio
async def test_overshoot_subtracted_next_step():
    vta = VirtualTimeAuthority(session=None, node_ids=[1])

    # Mock _get_reply
    Reply = namedtuple("Reply", ["ok"])
    Ok = namedtuple("Ok", ["payload"])
    Payload = namedtuple("Payload", ["to_bytes"])

    # First step: 10ms requested, but we simulate 10.002ms advanced (2000ns overshoot)
    mock_reply_1 = Reply(ok=Ok(payload=Payload(to_bytes=lambda: pack_clock_ready(10_002_000))))
    vta._get_reply = AsyncMock(return_value=mock_reply_1)

    await vta.step(10_000_000)

    assert vta._overshoot_ns[1] == 2_000
    assert vta.current_vtimes[1] == 10_002_000

    # Check that _get_reply was called with exactly 10_000_000
    call_args = vta._get_reply.call_args[0]
    payload_sent = call_args[2] # 3rd arg is payload
    delta_ns, mujoco_ns, _qn = struct.unpack("<QQQ", payload_sent)
    assert delta_ns == 10_000_000
    assert mujoco_ns == 10_000_000

    # Second step: 10ms requested, should request 10ms - 2000ns = 9_998_000 ns
    mock_reply_2 = Reply(ok=Ok(payload=Payload(to_bytes=lambda: pack_clock_ready(20_000_000))))
    vta._get_reply = AsyncMock(return_value=mock_reply_2)

    await vta.step(10_000_000)

    call_args = vta._get_reply.call_args[0]
    payload_sent = call_args[2]
    delta_ns, mujoco_ns, _qn = struct.unpack("<QQQ", payload_sent)
    assert delta_ns == 9_998_000
    assert mujoco_ns == 20_000_000 # 10_000_000 (expected from previous) + 10_000_000

    # After second step, since it returned exactly 20M, overshoot should be 0
    assert vta._overshoot_ns[1] == 0
    assert vta.current_vtimes[1] == 20_000_000

@pytest.mark.asyncio
async def test_overshoot_never_negative():
    vta = VirtualTimeAuthority(session=None, node_ids=[1])

    Reply = namedtuple("Reply", ["ok"])
    Ok = namedtuple("Ok", ["payload"])
    Payload = namedtuple("Payload", ["to_bytes"])

    # Request 10ms, mock returns 9ms (undershoot, shouldn't happen but clamp to 0)
    mock_reply = Reply(ok=Ok(payload=Payload(to_bytes=lambda: pack_clock_ready(9_000_000))))
    vta._get_reply = AsyncMock(return_value=mock_reply)

    await vta.step(10_000_000)

    assert vta._overshoot_ns[1] == 0
    assert vta.current_vtimes[1] == 9_000_000

@pytest.mark.asyncio
async def test_1000_quantum_drift_under_1_quantum():
    vta = VirtualTimeAuthority(session=None, node_ids=[1])

    Reply = namedtuple("Reply", ["ok"])
    Ok = namedtuple("Ok", ["payload"])
    Payload = namedtuple("Payload", ["to_bytes"])

    actual_sum_of_adjusted_deltas = 0
    current_mock_vtime = 0

    async def mock_get_reply(_nid, _topic, payload, _timeout):
        nonlocal actual_sum_of_adjusted_deltas, current_mock_vtime
        delta_ns, _mujoco_ns, qn = struct.unpack("<QQQ", payload)
        actual_sum_of_adjusted_deltas += delta_ns

        # QEMU executes adjusted delta + 100ns overshoot
        current_mock_vtime += delta_ns + 100

        return Reply(ok=Ok(payload=Payload(to_bytes=lambda: pack_clock_ready(current_mock_vtime, quantum_number=qn))))

    vta._get_reply = AsyncMock(side_effect=mock_get_reply)

    quantum_ns = 1_000_000 # 1ms

    for _ in range(1000):
        await vta.step(quantum_ns)

    expected_total_vtime = 1000 * quantum_ns

    assert vta._expected_vtime_ns[1] == expected_total_vtime

    drift = vta._expected_vtime_ns[1] - actual_sum_of_adjusted_deltas
    assert drift < quantum_ns

    # The actual sum should be: 1000 * 1ms - (999 * 100ns) = 1_000_000_000 - 99900 = 999_900_100
    assert actual_sum_of_adjusted_deltas == 999_900_100

