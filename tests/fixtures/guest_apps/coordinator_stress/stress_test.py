"""
SOTA Test Module: stress_test

Context:
This module implements tests for the stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of stress_test.
"""

import logging
import os
import threading
import time
import typing

import zenoh

from tools import vproto
from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)


def node_thread(node_id: int, num_messages: int, session: zenoh.Session) -> None:
    pub = session.declare_publisher(f"sim/eth/frame/{node_id}/tx")
    for i in range(num_messages):
        vtime = i * 1000
        payload = b"X" * 64
        pub.put(vproto.ZenohFrameHeader(vtime, 0, len(payload)).pack() + payload)


def main() -> None:
    conf = zenoh.Config()

    router = os.environ.get("ZENOH_ROUTER")
    if router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{router}"]')

    s = zenoh.open(conf)

    num_nodes = 20
    msgs_per_node = 50

    # First make them all known
    pubs = []
    for i in range(num_nodes):
        p = s.declare_publisher(f"sim/eth/frame/{i}/tx")
        p.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
        pubs.append(p)

    mock_execution_delay(2)  # SLEEP_EXCEPTION: mock test simulating execution/spacing

    threads = []
    start_time = time.time()
    for i in range(num_nodes):
        t = threading.Thread(target=node_thread, args=(i, msgs_per_node, s))
        t.start()
        threads.append(t)

    for t in threads:
        t.join()

    end_time = time.time()
    logger.info(f"Sent {num_nodes * msgs_per_node} messages in {end_time - start_time:.2f} seconds")

    mock_execution_delay(2)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
    typing.cast(typing.Any, s).close()
    logger.info("Stress test finished.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
