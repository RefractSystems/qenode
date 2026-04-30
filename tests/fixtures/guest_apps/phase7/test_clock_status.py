import logging
import sys
from pathlib import Path

import zenoh

SCRIPT_DIR = Path(Path(__file__).resolve().parent)
TOOLS_DIR = Path(Path(Path(SCRIPT_DIR).parent.parent)) / "tools"
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

logger = logging.getLogger(__name__)

TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 5.0


def pack_req(delta_ns):
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=0)
    return req.pack()


def unpack_rep(data):
    resp = ClockReadyResp.unpack(data)
    return resp.current_vtime_ns, resp.error_code


def main():
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--vtime", type=int, help="Wait for specific virtual time")
    parser.add_argument("router", nargs="?", default="tcp/127.0.0.1:7447", help="Zenoh router")
    args = parser.parse_args()

    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{args.router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    logger.info("Sending query...")
    replies = list(session.get(TOPIC, payload=pack_req(1000000), timeout=TIMEOUT_S))
    if not replies:
        logger.error("FAIL: No reply received")
        sys.exit(1)

    payload = replies[0].ok.payload.to_bytes()
    vtime, error_code = unpack_rep(payload)

    logger.info(f"Reply: vtime={vtime}, error_code={error_code}")

    if error_code == 0:
        logger.info("PASS: error_code is OK")
    else:
        logger.error(f"FAIL: Unexpected error_code {error_code} (1=STALL, 2=ZENOH_ERROR)")
        sys.exit(1)

    session.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
