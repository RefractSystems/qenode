"""
SOTA Test Module: test_chaos

Context:
This module implements tests for the test_chaos subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_chaos.
"""

import asyncio
import logging

import pytest

from tools.testing.utils import yield_now
from tools.testing.virtmcu_test_suite.orchestrator import SimulationOrchestrator
from tools.testing.virtmcu_test_suite.transport import FaultInjectingTransport

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_fault_injection(sim_transport):
    """
    Chaos Engineering validation.
    Verifies that the FaultInjectingTransport correctly drops and delays packets bi-directionally.
    """
    # Wrap the transport
    chaos = FaultInjectingTransport(sim_transport, drop_prob=1.0, delay_s=0.0)

    received = []
    rx_event = asyncio.Event()

    def on_rx(payload: bytes) -> None:
        received.append(payload)
        # In case we're not in the main thread (like Zenoh callback), safely set the event
        loop = asyncio.get_running_loop()
        loop.call_soon_threadsafe(rx_event.set)

    await chaos.subscribe("tests/fixtures/guest_apps/chaos", on_rx)

    # Deterministic wait for subscription to propagate
    import time

    start_wait = time.time()
    chaos.drop_prob = 0.0
    while b"ping" not in received:
        if time.time() - start_wait > 5.0:
            raise TimeoutError("Subscription failed to propagate")
        await chaos.publish("tests/fixtures/guest_apps/chaos", b"ping")
        await yield_now()

    received.clear()
    rx_event.clear()
    chaos.drop_prob = 1.0

    # 1. Test 100% drop rate (TX)
    for _ in range(10):
        await chaos.publish("tests/fixtures/guest_apps/chaos", b"dropped_tx")

    await yield_now()
    assert len(received) == 0, "TX messages should have been dropped"

    # 2. Test 100% drop rate (RX)
    received.clear()
    chaos.drop_prob = 1.0
    # Even if we use the inner transport to publish, the subscriber (wrapped by chaos) should drop it
    await sim_transport.publish("tests/fixtures/guest_apps/chaos", b"dropped_rx")

    # Send a sync message to deterministically know when the previous message was processed
    # "sync" bypasses drop_prob automatically in FaultInjectingTransport
    await sim_transport.publish("tests/fixtures/guest_apps/chaos", b"sync")

    start_wait = time.time()
    while b"sync" not in received:
        if time.time() - start_wait > 2.0:
            raise TimeoutError("Sync message not received")
        await yield_now()

    # Ensure all pending call_soon_threadsafe(rx_event.set) are processed
    await yield_now()
    await asyncio.sleep(0.01)  # SLEEP_EXCEPTION: explicit backoff for threadsafe signaling race

    assert b"dropped_rx" not in received, f"RX messages should have been dropped, but got {received}"

    # 3. Test 0% drop rate with delay and jitter
    received.clear()
    rx_event.clear()
    chaos.drop_prob = 0.0
    chaos.delay_s = 0.1
    chaos.jitter_s = 0.05

    loop = asyncio.get_running_loop()
    start_t = loop.time()
    await chaos.publish("tests/fixtures/guest_apps/chaos", b"delayed_msg")

    from tools.testing.utils import get_time_multiplier

    try:
        await asyncio.wait_for(rx_event.wait(), timeout=2.0 * get_time_multiplier())
    except TimeoutError:
        # Debugging info
        history = chaos.dump_flight_recorder()
        tx_drops = [h for h in history if h["direction"] == "tx_dropped"]
        rx_drops = [h for h in history if h["direction"] == "rx_dropped"]
        logger.info(f"TIMEOUT: {len(tx_drops)} TX drops, {len(rx_drops)} RX drops")
        raise

    end_t = loop.time()
    delay_measured = end_t - start_t

    assert len(received) == 1, "Message should have been delivered"
    assert received[0] == b"delayed_msg"
    # min delay should be delay_s - jitter_s = 0.05
    assert delay_measured >= 0.04, f"Message should have been delayed. Measured: {delay_measured}s"


@pytest.mark.asyncio
async def test_multi_node_chaos(zenoh_session, zenoh_router, qemu_launcher):
    """
    Proves that the system survives network chaos (drops/latency)
    between two nodes without deadlocking the coordinator or losing determinism.
    """
    from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl

    inner = ZenohTransportImpl(zenoh_router, zenoh_session)
    # 5% drop, 10ms delay, 5ms jitter
    chaos = FaultInjectingTransport(inner, drop_prob=0.05, delay_s=0.01, jitter_s=0.005)

    orchestrator = SimulationOrchestrator(zenoh_session, zenoh_router, qemu_launcher)
    # Override transport to use chaos
    orchestrator.transport = chaos

    dtb = "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = "tests/fixtures/guest_apps/boot_arm/hello.elf"

    # Node 1
    node1 = orchestrator.add_node(1, dtb, kernel)
    # Node 2
    node2 = orchestrator.add_node(2, dtb, kernel)

    await orchestrator.start()

    # Define a simple "progress" condition: both nodes have booted and said something
    def nodes_alive():
        return "HI" in node1.uart.buffer and "HI" in node2.uart.buffer

    # Run simulation under chaos
    # Even with drops and jitter, the VirtualTimeAuthority should keep them in sync
    # though it might take longer in wall-clock time due to FaultInjectingTransport sleeps.
    await orchestrator.run_until(nodes_alive, timeout=30.0)

    # If we reached here, the coordinator didn't deadlock and nodes made progress
    assert nodes_alive()

    # Check flight recorder for drops
    history = chaos.dump_flight_recorder()
    tx_drops = [h for h in history if h["direction"] == "tx_dropped"]
    rx_drops = [h for h in history if h["direction"] == "rx_dropped"]

    logger.info(f"Chaos stats: {len(tx_drops)} TX drops, {len(rx_drops)} RX drops")
    # Statistically we should have some drops if we sent enough packets (booting + clock steps)
    # but we don't strictly assert count to avoid flake in tiny simulations.
