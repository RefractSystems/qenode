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
from pathlib import Path

import zenoh
from vproto import ClockAdvanceReq, ClockReadyResp

logger = logging.getLogger(__name__)

# Add tools/ to path
def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent  # Fallback

SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE_DIR = _find_workspace_root(Path(__file__).resolve())
TOOLS_DIR = WORKSPACE_DIR / "tools"

if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))


def main():
    if len(sys.argv) <= 1:
        logger.error(f"Usage: {sys.argv[0]} <router_endpoint>")
        sys.exit(1)
    router = sys.argv[1]

    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

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
