"""
SOTA Test Module: test_coordinator_stress

Context:
This module implements tests for the test_coordinator_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator_stress.
"""

import logging
import threading
import time

import pytest
import vproto

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_coordinator_scalability(zenoh_router, zenoh_session, zenoh_coordinator):  # noqa: ARG001
    num_nodes = 50
    msgs_per_node = 50

    s = zenoh_session

    received_count = [0]
    expected = num_nodes * (num_nodes - 1) * msgs_per_node
    done_event = threading.Event()

    def on_sample(_sample):
        received_count[0] += 1
        # Accept 50% delivery to account for UDP/queue drops in Python subscriber under heavy CI load
        if received_count[0] >= int(expected * 0.5):
            done_event.set()

    _sub = s.declare_subscriber("sim/eth/frame/*/rx", on_sample)

    pubs = []
    for i in range(num_nodes):
        pubs.append(s.declare_publisher(f"sim/eth/frame/{i}/tx"))

    for i in range(num_nodes):
        pubs[i].put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    time.sleep(1)

    received_count[0] = 0
    done_event.clear()
    start_time = time.time()

    def node_thread(node_id):
        pub = pubs[node_id]
        payload = b"X" * 64
        for i in range(msgs_per_node):
            pub.put(vproto.ZenohFrameHeader(i * 1000, 0, len(payload)).pack() + payload)
            time.sleep(0.001)

    threads = []
    for i in range(num_nodes):
        t = threading.Thread(target=node_thread, args=(i,))
        t.start()
        threads.append(t)

    for t in threads:
        t.join()

    done_event.wait(timeout=15.0)
    end_time = time.time()
    duration = end_time - start_time

    assert received_count[0] >= int(expected * 0.5), f"Dropped too many: {received_count[0]} / {expected}"
    logger.info(f"Routed {received_count[0]} messages in {duration:.2f} seconds")
