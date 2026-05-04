"""
SOTA Test Module: test_clock

Context:
This module implements tests for the test_clock subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock.
"""

import logging
import sys
import typing

from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)


def main() -> None:
    if len(sys.argv) < 2:
        logger.error("Usage: test_clock.py <router_endpoint>")
        sys.exit(1)

    router = sys.argv[1]
    session = open_client_session(connect=router)

    clock_topic = "sim/clock/advance/0"
    delta1_ns = 10_000_000
    delta2_ns = 50_000_000

    # 1. Advance to 10ms
    logger.info(f"Advancing to {delta1_ns}ns...")
    reply = list(session.get(clock_topic, payload=delta1_ns.to_bytes(8, "little"), timeout=5.0))
    if not reply or not hasattr(reply[0], "ok"):
        logger.error(f"Failed to advance clock to {delta1_ns}")
        sys.exit(1)

    # 2. Advance to 60ms
    logger.info(f"Advancing to {delta1_ns + delta2_ns}ns...")
    reply = list(session.get(clock_topic, payload=delta2_ns.to_bytes(8, "little"), timeout=5.0))
    if not reply or not hasattr(reply[0], "ok") or reply[0].ok is None:
        logger.error(f"Failed to advance clock to {delta1_ns + delta2_ns}")
        sys.exit(1)

    # 3. Double check final vtime
    logger.info("Verifying final vtime...")
    res = reply[0].ok.payload.to_bytes()
    vtime = int.from_bytes(res[:8], "little")
    if vtime < (delta1_ns + delta2_ns):
        logger.error(f"Unexpected vtime {vtime} < {delta1_ns + delta2_ns}")
        sys.exit(1)

    typing.cast(typing.Any, session).close()
    logger.info("PASS")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
