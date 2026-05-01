"""
SOTA Test Module: zenoh_router_persistent

Context:
This module implements tests for the zenoh_router_persistent subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of zenoh_router_persistent.
"""

import logging
import sys
import time

import zenoh

logger = logging.getLogger(__name__)


def main():
    if len(sys.argv) <= 1:
        sys.exit(1)
    endpoint = sys.argv[1]
    config = zenoh.Config()
    config.insert_json5("mode", '"router"')
    config.insert_json5("listen/endpoints", f'["{endpoint}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    import contextlib

    with contextlib.suppress(Exception):
        config.insert_json5("transport/shared/task_workers", "16")
    logger.info(f"Starting persistent Zenoh mock router on {endpoint}...")
    session = zenoh.open(config)

    logger.info("Zenoh router started. Declaring liveliness...")
    _liveliness = session.liveliness().declare_token("sim/router/check")
    logger.info("Liveliness declared. Ready.")

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    session.close()


if __name__ == "__main__":
    main()
