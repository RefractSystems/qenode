"""
SOTA Test Module: repro_crash

Context:
This module implements tests for the repro_crash subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of repro_crash.
"""

import logging
import os
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    router = os.environ.get("ZENOH_ROUTER", "tcp/localhost:7447")
    s = open_client_session(connect=router)
    logger.info("Connected.")
    # Send malformed data
    pub = s.declare_publisher("sim/clock/advance/0")
    pub.put(b"MALFORMED")
    typing.cast(typing.Any, s).close()
    logger.info("Done.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
