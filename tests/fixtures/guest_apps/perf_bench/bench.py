"""
SOTA Test Module: bench

Context:
This module implements tests for the bench subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of bench.
"""

import argparse
import logging
import os
import time
import typing

import numpy as np

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


class BenchRunner:
    def __init__(self, router: str, topic: str, count: int, payload_size: int) -> None:
        self.router = router
        self.topic = topic
        self.count = count
        self.payload_size = payload_size
        self.rtts: list[float] = []

    def run(self) -> None:
        session = open_client_session(connect=self.router)
        payload = b"X" * self.payload_size

        logger.info(f"Starting benchmark: {self.count} gets on {self.topic}...")

        for _ in range(self.count):
            t0 = time.perf_counter()
            reply = list(session.get(self.topic, payload=payload, timeout=5.0))
            t1 = time.perf_counter()
            if not reply:
                logger.error("No reply received!")
            self.rtts.append(t1 - t0)

        typing.cast(typing.Any, session).close()

    def report(self) -> None:
        if not self.rtts:
            return
        rtts_ms = np.array(self.rtts) * 1000
        logger.info(f"Count: {len(self.rtts)}")
        logger.info(f"Mean:   {np.mean(rtts_ms):.3f} ms")
        logger.info(f"Min:    {np.min(rtts_ms):.3f} ms")
        logger.info(f"Max:    {np.max(rtts_ms):.3f} ms")
        logger.info(f"P99:    {np.percentile(rtts_ms, 99):.3f} ms")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--router", default=os.environ.get("ZENOH_ROUTER", "tcp/localhost:7447"))
    parser.add_argument("--topic", default="sim/clock/advance/0")
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--size", type=int, default=8)
    args = parser.parse_args()

    runner = BenchRunner(args.router, args.topic, args.count, args.size)
    runner.run()
    runner.report()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
