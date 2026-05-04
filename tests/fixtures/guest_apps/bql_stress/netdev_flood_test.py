"""
SOTA Test Module: netdev_flood_test

Context:
This module implements tests for the netdev_flood_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of netdev_flood_test.
"""

import logging
import sys
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)

if len(sys.argv) <= 1:
    sys.exit(1)
router = sys.argv[1]
session = open_client_session(connect=router)
logger.info("[Flood] Connected to Zenoh.")

pub = session.declare_publisher("virtmcu/netdev/0/tx")
for _i in range(1000):
    # Header (24 bytes) + large ethernet packet (1500 bytes)
    payload = b"\x00" * 24 + b"X" * 1500
    pub.put(payload)

logger.info("[Flood] Finished publishing 1000 packets.")
typing.cast(typing.Any, session).close()
