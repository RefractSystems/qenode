"""
SOTA Test Module: netdev_stress_test

Context:
This module implements tests for the netdev_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of netdev_stress_test.
"""

import argparse
import logging
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--router", required=True)
    parser.add_argument("--topic", required=True)
    parser.add_argument("--count", type=int, default=1000)
    args = parser.parse_args()

    session = open_client_session(connect=args.router)
    pub = session.declare_publisher(args.topic)

    logger.info(f"Connected to {args.router}, blasting {args.count} packets to {args.topic}...")

    for i in range(args.count):
        # Header (24 bytes) + data
        payload = b"\x00" * 24 + f"STRESS {i}".encode()
        pub.put(payload)

    logger.info("Stress test complete.")
    typing.cast(typing.Any, session).close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
