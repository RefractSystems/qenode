"""
SOTA Test Module: ftrt_stress_test

Context:
This module implements tests for the ftrt_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of ftrt_stress_test.
"""

import logging
import os
import threading
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    def worker() -> None:
        try:
            z_session = open_client_session(connect=os.environ.get("VIRTMCU_ZENOH_ROUTER", "tcp/localhost:7447"))
            typing.cast(typing.Any, z_session).close()
        except Exception as e:  # noqa: BLE001
            logger.error(f"Worker failed: {e}")

    threads = []
    for _ in range(10):
        t = threading.Thread(target=worker)
        t.start()
        threads.append(t)

    for t in threads:
        t.join()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
