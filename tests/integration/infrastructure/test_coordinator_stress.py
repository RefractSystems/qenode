# ZENOH_HACK_EXCEPTION: Tests zenoh_coordinator natively by mocking QEMU nodes
"""
SOTA Test Module: test_coordinator_stress

Context:
This module implements tests for the test_coordinator_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator_stress.
"""

from __future__ import annotations

import asyncio
import logging
import time
from collections.abc import Callable
from typing import TYPE_CHECKING, Any, cast

import pytest
import zenoh

from tools import vproto
from tools.testing.utils import get_time_multiplier
from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary
from tools.testing.virtmcu_test_suite.conftest_core import coordinator_subprocess
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    pass


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_coordinator_scalability(zenoh_router: str, zenoh_session: zenoh.Session) -> None:
    """
    Smoke test the coordinator with a small number of nodes and messages.
    Ensures the PDES barrier progresses and delivers messages.
    """
    coordinator_bin = resolve_rust_binary(VirtmcuBinary.DETERMINISTIC_COORDINATOR)
    num_nodes = 3
    msgs_per_node = 5
    s = zenoh_session

    received_count = [0]
    expected = num_nodes * (num_nodes - 1) * msgs_per_node
    done_event = asyncio.Event()
    loop = asyncio.get_running_loop()

    def on_sample(_sample: zenoh.Sample) -> None:
        received_count[0] += 1
        if received_count[0] >= expected:
            loop.call_soon_threadsafe(done_event.set)

    _sub = s.declare_subscriber(SimTopic.ETH_FRAME_RX_WILDCARD, on_sample)

    pubs = [s.declare_publisher(SimTopic.eth_tx(i)) for i in range(num_nodes)]
    done_pubs = [s.declare_publisher(SimTopic.coord_done(i)) for i in range(num_nodes)]

    start_queues: list[asyncio.Queue[bytes]] = [asyncio.Queue() for _ in range(num_nodes)]

    def make_on_start(nid: int) -> Callable[[zenoh.Sample], None]:
        def _callback(sample: zenoh.Sample) -> None:
            loop.call_soon_threadsafe(start_queues[nid].put_nowait, sample.payload.to_bytes())

        return _callback

    start_subs = [s.declare_subscriber(SimTopic.clock_start(i), make_on_start(i)) for i in range(num_nodes)]

    # Create a simple topology file to avoid default fallback issues
    import tempfile

    import yaml

    from tools.testing.virtmcu_test_suite.generated import Node, NodeID, Protocol, Topology, WireLink, World

    world = World(
        topology=Topology(
            nodes=[Node(name=NodeID(root=str(i))) for i in range(num_nodes)],
            global_seed="42",
            links=[WireLink(type=Protocol(root="ethernet"), nodes=[NodeID(root=str(i)) for i in range(num_nodes)])],
        )
    )
    with tempfile.NamedTemporaryFile(mode="w", suffix=".yaml", delete=False) as f:
        f.write(yaml.dump(world.model_dump(exclude_none=True, by_alias=True), sort_keys=False))
        topo_path = f.name

    async with coordinator_subprocess(
        binary=coordinator_bin,
        args=["--connect", zenoh_router, "--topology", topo_path],
        zenoh_session=s,
    ):
        start_time = time.time()

        async def mock_node(node_id: int) -> None:
            pub = pubs[node_id]
            done_pub = done_pubs[node_id]
            q = start_queues[node_id]

            payload = b"X" * 64
            quantum = 0
            for i in range(msgs_per_node):
                quantum += 1
                # 1. Send message for CURRENT quantum
                pub.put(vproto.ZenohFrameHeader(quantum * 1000, 0, len(payload)).pack() + payload)

                # 2. Signal DONE for CURRENT quantum
                done_pub.put(quantum.to_bytes(8, "little"))
                # 3. Wait for NEXT quantum start
                if i < msgs_per_node - 1:
                    try:
                        await asyncio.wait_for(q.get(), timeout=5.0 * get_time_multiplier())
                    except TimeoutError:
                        logger.error(f"Node {node_id} timed out waiting for quantum {quantum + 1}")
                        break

            # Keep pumping DONEs to flush any delayed TX messages due to Zenoh topic racing
            while not done_event.is_set():
                try:
                    await asyncio.wait_for(q.get(), timeout=0.1)
                    quantum += 1
                    done_pub.put(quantum.to_bytes(8, "little"))
                except TimeoutError:
                    pass

        # Run all mock nodes concurrently
        await asyncio.gather(*(mock_node(i) for i in range(num_nodes)))

        # Wait for delivery
        timeout = 5.0 * get_time_multiplier()
        try:
            await asyncio.wait_for(done_event.wait(), timeout=timeout)
        except TimeoutError:
            pass

        end_time = time.time()
        duration = end_time - start_time

    assert received_count[0] >= expected, f"Dropped too many: {received_count[0]} / {expected}"
    logger.info(f"Routed {received_count[0]} messages in {duration:.2f} seconds")

    for sub in start_subs:
        cast(Any, sub).undeclare()
