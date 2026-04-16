import sys
import struct
import zenoh

DELTA1_NS = 1_000_000
DELTA2_NS = 2_000_000
TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 5.0


def pack_req(delta_ns):
    return struct.pack("<QQ", delta_ns, 0)


def unpack_rep(data):
    # New payload is 16 bytes: <Q (vtime_ns) I (status) I (n_frames)
    vtime_ns, status, _n_frames = struct.unpack("<QII", data)
    if status != 0:
        print(f"WARNING: Reply status = {status}", file=sys.stderr)
    return vtime_ns


def send_query(session, delta_ns, label):
    replies = list(session.get(TOPIC, payload=pack_req(delta_ns), timeout=TIMEOUT_S))
    if not replies:
        print(f"{label}: TIMEOUT — no reply received", file=sys.stderr)
        sys.exit(1)
    reply = replies[0]
    if not hasattr(reply, "ok"):
        print(f"{label}: ERROR reply: {reply}", file=sys.stderr)
        sys.exit(1)
    return unpack_rep(reply.ok.payload.to_bytes())


def main():
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7447"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    vtime1 = send_query(session, DELTA1_NS, "Q1")
    print(f"Q1 vtime = {vtime1} ns  (expected >= {DELTA1_NS})")
    if vtime1 < DELTA1_NS:
        print(f"FAIL: Q1 vtime {vtime1} < expected {DELTA1_NS}", file=sys.stderr)
        sys.exit(1)

    vtime2 = send_query(session, DELTA2_NS, "Q2")
    print(f"Q2 vtime = {vtime2} ns  (expected >= {DELTA1_NS + DELTA2_NS})")
    if vtime2 <= vtime1:
        print(f"FAIL: Q2 vtime {vtime2} not > Q1 vtime {vtime1}", file=sys.stderr)
        sys.exit(1)
    if vtime2 < DELTA1_NS + DELTA2_NS:
        print(f"FAIL: Q2 vtime {vtime2} < cumulative {DELTA1_NS + DELTA2_NS}", file=sys.stderr)
        sys.exit(1)

    session.close()
    print("PASS")


if __name__ == "__main__":
    main()
