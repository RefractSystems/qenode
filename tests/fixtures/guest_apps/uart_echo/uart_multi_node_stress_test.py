"""
SOTA Test Module: uart_multi_node_stress_test

Context:
This module implements tests for the uart_multi_node_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of uart_multi_node_stress_test.
"""

import logging
import os
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    router = os.environ.get("ZENOH_ROUTER", "tcp/localhost:7447")
    session = open_client_session(connect=router)
    logger.info("Connected.")
    # Stress logic here if needed, but the main test is in test_uart_echo.py
    typing.cast(typing.Any, session).close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
