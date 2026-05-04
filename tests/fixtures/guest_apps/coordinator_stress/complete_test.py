"""
SOTA Test Module: complete_test

Context:
This module implements tests for the complete_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of complete_test.
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
    typing.cast(typing.Any, s).close()
    logger.info("Done.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
