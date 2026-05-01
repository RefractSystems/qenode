"""
SOTA Test Module: test_overflow

Context:
This module implements tests for the test_overflow subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_overflow.
"""

import logging
import sys
import time

import vproto
import zenoh

logger = logging.getLogger(__name__)


def main():
    conf = zenoh.Config()
    s = zenoh.open(conf)

    rx_frames = []

    def on_rx(sample):
        rx_frames.append(sample.payload.to_bytes())

    s.declare_subscriber("sim/eth/frame/2/rx", on_rx)

    pub1 = s.declare_publisher("sim/eth/frame/1/tx")
    pub2 = s.declare_publisher("sim/eth/frame/2/tx")

    time.sleep(1)
    pub2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    time.sleep(0.5)

    orig_vtime = 0xFFFFFFFFFFFFFFFF - 500000
    pub1.put(vproto.ZenohFrameHeader(orig_vtime, 0, 4).pack() + b"DEAD")

    time.sleep(1)

    if len(rx_frames) == 0:
        logger.error("FAIL: No frame received")
        sys.exit(1)

    vtime = int.from_bytes(rx_frames[0][:8], "little")
    _size = int.from_bytes(rx_frames[0][8:12], "little")
    logger.info(f"Original vtime: {orig_vtime}")
    logger.info(f"Forwarded vtime: {vtime}")

    if vtime < orig_vtime:
        logger.error("FAIL: VTime wrapped around!")
        sys.exit(1)
    else:
        logger.info("PASS: VTime did not wrap around.")

    s.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
