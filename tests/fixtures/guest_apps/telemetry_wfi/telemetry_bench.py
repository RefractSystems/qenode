"""
SOTA Test Module: telemetry_bench

Context:
This module implements tests for the telemetry_bench subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of telemetry_bench.
"""

import logging
import os
import sys
import time
import typing

import numpy as np
import zenoh

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    router_url = os.environ.get("ZENOH_ROUTER", "tcp/localhost:7447")
    session = open_client_session(connect=router_url)

    topic = "telemetry/0/value"
    count = 1000
    received_times: list[float] = []

    def on_sample(sample: zenoh.Sample) -> None:
        received_times.append(time.perf_counter())

    sub = session.declare_subscriber(topic, on_sample)

    logger.info(f"Subscribed to {topic}, waiting for {count} samples...")

    # Wait for samples
    deadline = time.perf_counter() + 60.0
    while len(received_times) < count and time.perf_counter() < deadline:
        time.sleep(0.1)  # SLEEP_EXCEPTION: Intentional delay for bench

    typing.cast(typing.Any, sub).undeclare()
    typing.cast(typing.Any, session).close()

    if len(received_times) < count:
        logger.error(f"Timed out. Received {len(received_times)}/{count} samples.")
        sys.exit(1)

    # Calculate intervals
    intervals = np.diff(received_times) * 1000
    logger.info(f"Mean Interval: {np.mean(intervals):.3f} ms")
    logger.info(f"P99 Interval:   {np.percentile(intervals, 99):.3f} ms")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
