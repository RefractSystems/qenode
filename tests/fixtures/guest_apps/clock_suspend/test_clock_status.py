"""
SOTA Test Module: test_clock_status

Context:
This module implements tests for the test_clock_status subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock_status.
"""

import argparse
import logging
import sys
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--router", required=True)
    parser.add_argument("--expected-error", type=int, default=0)
    args = parser.parse_args()

    session = open_client_session(connect=args.router)

    clock_topic = "sim/clock/advance/0"
    delta_ns = 1_000_000

    # Advance clock
    reply = list(session.get(clock_topic, payload=delta_ns.to_bytes(8, "little"), timeout=5.0))
    if not reply or not hasattr(reply[0], "ok") or reply[0].ok is None:
        logger.error("Failed to get reply from clock")
        sys.exit(1)

    res = reply[0].ok.payload.to_bytes()
    if len(res) < 12:
        logger.error(f"Reply too short: {len(res)}")
        sys.exit(1)

    error_code = int.from_bytes(res[8:12], "little")
    if error_code != args.expected_error:
        logger.error(f"Unexpected error_code {error_code} (expected {args.expected_error})")
        sys.exit(1)

    typing.cast(typing.Any, session).close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
