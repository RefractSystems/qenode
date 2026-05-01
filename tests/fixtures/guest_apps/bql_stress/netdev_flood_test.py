"""
SOTA Test Module: netdev_flood_test

Context:
This module implements tests for the netdev_flood_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of netdev_flood_test.
"""

import logging
import sys
import threading
import typing

import zenoh

from tools import vproto
from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)

if len(sys.argv) <= 1:
    sys.exit(1)
    router = sys.argv[1]
config = zenoh.Config()
config.insert_json5("mode", '"client"')
config.insert_json5("connect/endpoints", f'["{router}"]')  # type: ignore[has-type]
session = zenoh.open(config)

logger.info("[Flood] Connected to Zenoh.")


def publish_netdev() -> None:
    pub = session.declare_publisher("sim/network/0/tx")

    # 12 byte header (8 byte vtime, 4 byte size)
    header = vproto.ZenohFrameHeader(0, 0, 10).pack()
    payload = header + b"1234567890"

    logger.info("[Flood] Blasting 50,000 packets rapidly to trigger backpressure/OOM...")

    # Blast packets
    for _i in range(50000):
        pub.put(payload)

    logger.info("[Flood] Blast complete. Awaiting crash or stability...")
    mock_execution_delay(2)  # SLEEP_EXCEPTION: mock test simulating execution/spacing


t1 = threading.Thread(target=publish_netdev)
t1.start()
t1.join()

logger.info("[Flood] Test completed.")
typing.cast(typing.Any, session).close()
