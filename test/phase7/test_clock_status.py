import os
import sys

import zenoh

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
TOOLS_DIR = os.path.join(os.path.dirname(os.path.dirname(SCRIPT_DIR)), "tools")
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 5.0


def pack_req(delta_ns):
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0)
    return req.pack()


def unpack_rep(data):
    resp = ClockReadyResp.unpack(data)
    return resp.current_vtime_ns, resp.error_code


def main():
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7447"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    print("Sending query...")
    replies = list(session.get(TOPIC, payload=pack_req(1000000), timeout=TIMEOUT_S))
    if not replies:
        print("FAIL: No reply received", file=sys.stderr)
        sys.exit(1)

    payload = replies[0].ok.payload.to_bytes()
    vtime, error_code = unpack_rep(payload)

    print(f"Reply: vtime={vtime}, error_code={error_code}")

    if error_code == 0:
        print("PASS: error_code is OK")
    else:
        print(f"FAIL: Unexpected error_code {error_code} (1=STALL, 2=ZENOH_ERROR)", file=sys.stderr)
        sys.exit(1)

    session.close()


if __name__ == "__main__":
    main()
