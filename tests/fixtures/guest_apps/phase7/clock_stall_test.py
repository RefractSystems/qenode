import logging
import sys
import time

import zenoh

logger = logging.getLogger(__name__)

router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

logger.info("[Stall Test] Connected to Zenoh.")


def handle_advance(query):
    logger.info(f"[Stall Test] Received clock advance request: {query.selector}")
    logger.info("[Stall Test] Purposely sleeping for 6 seconds to trigger QEMU stall_timeout_ms=5000...")
    time.sleep(6.0)

    # Reply after timeout just to see if QEMU crashed or exited cleanly
    query.reply(query.selector, b"\x00" * 16)
    logger.info("[Stall Test] Sent late reply.")


sub = session.declare_queryable("sim/clock/advance/0", handle_advance)

logger.info("[Stall Test] Listening for advance requests...")
time.sleep(10)
session.close()
