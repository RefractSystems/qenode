import sys
import struct
import zenoh
import time

TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 5.0

def pack_req(delta_ns):
    return struct.pack("<QQ", delta_ns, 0)

def unpack_rep(data):
    # Expect 16 bytes: <Q (vtime_ns) I (status) I (n_frames)
    if len(data) != 16:
        print(f"ERROR: Expected 16 bytes, got {len(data)}", file=sys.stderr)
        sys.exit(1)
    vtime_ns, status, n_frames = struct.unpack("<QII", data)
    return vtime_ns, status

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
    vtime, status = unpack_rep(payload)
    
    print(f"Reply: vtime={vtime}, status={status}")
    
    if status == 0:
        print("PASS: status is OK")
    else:
        print(f"FAIL: Unexpected status {status}", file=sys.stderr)
        sys.exit(1)

    session.close()

if __name__ == "__main__":
    main()
