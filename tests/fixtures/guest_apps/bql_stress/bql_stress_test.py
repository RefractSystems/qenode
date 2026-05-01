"""
SOTA Test Module: bql_stress_test

Context:
This module implements tests for the bql_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of bql_stress_test.
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
logger.info("[Stress] Connected to Zenoh.")


def publish_chardev() -> None:
    pub = session.declare_publisher("virtmcu/uart/0/rx")
    for _i in range(1000):
        # 12 byte header (8 byte vtime, 4 byte size) + payload

        header = vproto.ZenohFrameHeader(0, 0, 5).pack()
        payload = header + b"Hello"
        pub.put(payload)
        mock_execution_delay(0.001)  # SLEEP_EXCEPTION: mock test simulating execution/spacing


def publish_ui() -> None:
    pub = session.declare_publisher("sim/ui/0/button/1")
    for i in range(1000):
        pub.put(b"\x01" if i % 2 == 0 else b"\x00")
        mock_execution_delay(0.001)  # SLEEP_EXCEPTION: mock test simulating execution/spacing


t1 = threading.Thread(target=publish_chardev)
t2 = threading.Thread(target=publish_ui)

t1.start()
t2.start()

t1.join()
t2.join()

logger.info("[Stress] Finished publishing 2000 events.")
typing.cast(typing.Any, session).close()
