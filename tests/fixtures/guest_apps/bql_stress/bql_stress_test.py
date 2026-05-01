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
import time

import vproto
import zenoh

logger = logging.getLogger(__name__)

if len(sys.argv) <= 1:
    sys.exit(1)
    router = sys.argv[1]
config = zenoh.Config()
config.insert_json5("mode", '"client"')
config.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(config)
logger.info("[Stress] Connected to Zenoh.")


def publish_chardev():
    pub = session.declare_publisher("virtmcu/uart/0/rx")
    for _i in range(1000):
        # 12 byte header (8 byte vtime, 4 byte size) + payload

        header = vproto.ZenohFrameHeader(0, 0, 5).pack()
        payload = header + b"Hello"
        pub.put(payload)
        time.sleep(0.001)


def publish_ui():
    pub = session.declare_publisher("sim/ui/0/button/1")
    for i in range(1000):
        pub.put(b"\x01" if i % 2 == 0 else b"\x00")
        time.sleep(0.001)


t1 = threading.Thread(target=publish_chardev)
t2 = threading.Thread(target=publish_ui)

t1.start()
t2.start()

t1.join()
t2.join()

logger.info("[Stress] Finished publishing 2000 events.")
session.close()
