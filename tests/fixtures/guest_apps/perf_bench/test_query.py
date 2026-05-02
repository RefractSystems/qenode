"""
SOTA Test Module: test_query

Context:
This module implements tests for the test_query subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_query.
"""

import logging
import sys
import time

import zenoh

from tools.vproto import ClockAdvanceReq, ClockReadyResp

logger = logging.getLogger(__name__)


# Add tools/ to path
def main() -> None:
    if len(sys.argv) <= 1:
        logger.error(f"Usage: {sys.argv[0]} <router_endpoint>")
        sys.exit(1)
    router = sys.argv[1]

    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    from typing import Any

    session: Any = zenoh.open(config)

    topic = "sim/clock/advance/0"
    logger.info(f"Sending query to {topic}...")

    req = ClockAdvanceReq(delta_ns=1000000, mujoco_time_ns=0, quantum_number=0).pack()

    start = time.perf_counter()
    replies = list(session.get(topic, payload=req, timeout=10.0))
    end = time.perf_counter()

    if not replies:
        logger.info("No replies received!")
    else:
        for reply in replies:
            if reply.ok:
                resp = ClockReadyResp.unpack(reply.ok.payload.to_bytes())
                logger.info(f"Reply: vtime={resp.current_vtime_ns}, error={resp.error_code}")
            else:
                logger.error(f"Error reply: {reply.err}")

    logger.info(f"Query took {end - start:.3f}s")
    session.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
