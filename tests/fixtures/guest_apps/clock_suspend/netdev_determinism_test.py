"""
SOTA Test Module: netdev_determinism_test

Context:
This module implements tests for the netdev_determinism_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of netdev_determinism_test.
"""

import argparse
import logging
import sys
import typing

import zenoh

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--router", required=True)
    parser.add_argument("--topic", required=True)
    parser.add_argument("--count", type=int, default=100)
    args = parser.parse_args()

    session = open_client_session(connect=args.router)
    received_count = 0

    def on_sample(sample: zenoh.Sample) -> None:
        nonlocal received_count
        received_count += 1

    sub = session.declare_subscriber(args.topic, on_sample)

    # We just wait for messages to come in. The simulation drives time.
    try:
        import time

        deadline = time.perf_counter() + 30.0
        while received_count < args.count and time.perf_counter() < deadline:
            time.sleep(0.1)  # SLEEP_EXCEPTION: Intentional delay for determinism test
    finally:
        typing.cast(typing.Any, sub).undeclare()
        typing.cast(typing.Any, session).close()

    if received_count < args.count:
        logger.error(f"Timed out. Received {received_count}/{args.count} samples.")
        sys.exit(1)

    logger.info(f"Successfully received {received_count} samples.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
