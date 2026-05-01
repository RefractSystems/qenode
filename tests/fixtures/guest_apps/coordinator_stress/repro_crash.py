"""
SOTA Test Module: repro_crash

Context:
This module implements tests for the repro_crash subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of repro_crash.
"""

import logging
import os
import queue
import time

import vproto
import zenoh

logger = logging.getLogger(__name__)


def main():
    conf = zenoh.Config()

    router = os.environ.get("ZENOH_ROUTER")
    if router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{router}"]')

    s = zenoh.open(conf)
    pub = s.declare_publisher("sim/eth/frame/malicious/tx")

    logger.info("Sending malformed packet (too short)...")
    pub.put(b"\x00\x01\x02")  # Only 3 bytes, header expects 12

    time.sleep(0.5)

    # If coordinator is still alive, this should work
    logger.info("Sending valid packet to check if coordinator is alive...")
    pub_valid = s.declare_publisher("sim/eth/frame/1/tx")

    rx_valid = queue.Queue()
    s.declare_subscriber("sim/eth/frame/2/rx", lambda s: rx_valid.put(s.payload.to_bytes()))

    # Node 2 must be "known"
    pub2 = s.declare_publisher("sim/eth/frame/2/tx")
    pub2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    time.sleep(0.5)

    pub_valid.put(vproto.ZenohFrameHeader(1000, 0, 4).pack() + b"ABCD")

    timeout_val = 5.0
    if os.environ.get("VIRTMCU_USE_ASAN") == "1":
        timeout_val = 50.0

    try:
        rx_valid.get(timeout=timeout_val)
        logger.info("Received valid packet")
    except queue.Empty:
        logger.info("Timeout waiting for valid packet, coordinator might have crashed")
        s.close()
        exit(1)

    logger.info("Test finished. Check coordinator logs.")
    s.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
