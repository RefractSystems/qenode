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

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)

if len(sys.argv) <= 1:
    sys.exit(1)
router = sys.argv[1]
session = open_client_session(connect=router)
logger.info("[Stress] Connected to Zenoh.")


def publish_chardev() -> None:
    pub = session.declare_publisher("virtmcu/chardev/0")
    for i in range(1000):
        # Header (24 bytes) + data
        payload = b"\x00" * 24 + f"CHR {i}\n".encode()
        pub.put(payload)
    logger.info("[Stress] CHR thread done.")


def publish_ui() -> None:
    pub = session.declare_publisher("virtmcu/ui/0")
    for i in range(1000):
        # Header (24 bytes) + data
        payload = b"\x00" * 24 + f"UI {i}\n".encode()
        pub.put(payload)
    logger.info("[Stress] UI thread done.")


t1 = threading.Thread(target=publish_chardev)
t2 = threading.Thread(target=publish_ui)

t1.start()
t2.start()

t1.join()
t2.join()

logger.info("[Stress] Finished publishing 2000 events.")
typing.cast(typing.Any, session).close()
