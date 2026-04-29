import asyncio

import pytest

from tools.testing.utils import yield_now
from tools.testing.virtmcu_test_suite.transport import FaultInjectingTransport


@pytest.mark.asyncio
async def test_det3_fault_injection(sim_transport):
    """
    DET-3: Chaos Engineering validation.
    Verifies that the FaultInjectingTransport correctly drops and delays packets.
    """
    # Wrap the transport
    chaos = FaultInjectingTransport(sim_transport, drop_prob=1.0, delay_s=0.0)

    received = []
    rx_event = asyncio.Event()

    def on_rx(payload):
        received.append(payload)
        rx_event.set()

    await chaos.subscribe("test/chaos", on_rx)

    # 1. Test 100% drop rate
    for _ in range(10):
        await chaos.publish("test/chaos", b"dropped_msg")

    await yield_now()
    assert len(received) == 0, "Messages should have been dropped by Chaos Transport"

    # 2. Test 0% drop rate with delay
    chaos.drop_prob = 0.0
    chaos.delay_s = 0.1

    loop = asyncio.get_running_loop()
    start_t = loop.time()
    await chaos.publish("test/chaos", b"delayed_msg")

    from tools.testing.utils import get_time_multiplier

    await asyncio.wait_for(rx_event.wait(), timeout=1.0 * get_time_multiplier())

    end_t = loop.time()

    assert len(received) == 1, "Message should have been delivered"
    assert received[0] == b"delayed_msg"
    assert (end_t - start_t) >= 0.1, "Message should have been delayed by at least 100ms"
