import os
import sys

import zenoh

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
TOOLS_DIR = os.path.join(os.path.dirname(os.path.dirname(SCRIPT_DIR)), "tools")
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

DELTA1_NS = 1_000_000
DELTA2_NS = 2_000_000
TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 10.0


def pack_req(delta_ns):
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0)
    return req.pack()


def unpack_rep(data):
    resp = ClockReadyResp.unpack(data)
    if resp.error_code != 0:
        print(f"WARNING: Reply error_code = {resp.error_code} (1=STALL, 2=ZENOH_ERROR)", file=sys.stderr)
    return resp.current_vtime_ns


def send_query(session, delta_ns, label):
    replies = list(session.get(TOPIC, payload=pack_req(delta_ns), timeout=TIMEOUT_S))
    if not replies:
        print(f"{label}: TIMEOUT — no reply received", file=sys.stderr)
        sys.exit(1)
    reply = replies[0]
    if getattr(reply, "err", None) is not None:
        print(f"{label}: ERROR reply: {reply.err}", file=sys.stderr)
        sys.exit(1)
    if not hasattr(reply, "ok"):
        print(f"{label}: NO 'ok' in reply: {reply}", file=sys.stderr)
        sys.exit(1)
    if reply.ok is None:
        print(f"{label}: reply.ok IS NONE. Full reply: {reply}", file=sys.stderr)
        sys.exit(1)
    return unpack_rep(reply.ok.payload.to_bytes())


def main():
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7447"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    vtime1 = send_query(session, DELTA1_NS, "Q1")
    print(f"Q1 vtime = {vtime1} ns")

    vtime2 = send_query(session, DELTA2_NS, "Q2")
    print(f"Q2 vtime = {vtime2} ns  (target approx {vtime1 + DELTA1_NS})")
    if vtime2 < vtime1:
        print(f"FAIL: Q2 vtime {vtime2} < Q1 vtime {vtime1}", file=sys.stderr)
        sys.exit(1)

    vtime3 = send_query(session, 1_000_000, "Q3")
    print(f"Q3 vtime = {vtime3} ns  (target approx {vtime2 + DELTA2_NS})")
    
    if vtime3 < vtime2 + DELTA2_NS:
        print(f"FAIL: Q3 vtime {vtime3} < Q2 vtime {vtime2} + DELTA2 {DELTA2_NS}", file=sys.stderr)
        sys.exit(1)

    session.close()
    print("PASS")


if __name__ == "__main__":
    main()
