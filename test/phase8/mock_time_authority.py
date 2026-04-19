import sys
import struct

import zenoh


# Standard virtmcu ClockAdvanceReq/ClockReadyResp packing
def pack_clock_advance(delta_ns, mujoco_time_ns=0):
    return struct.pack("<QQ", delta_ns, mujoco_time_ns)

def unpack_clock_ready(data):
    # current_vtime_ns (Q), n_frames (I), error_code (I)
    return struct.unpack("<QII", data)

def main():
    router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    session = zenoh.open(conf)

    print("[TimeAuthority] Advancing clock on sim/clock/advance/0...")

    # Advance 2 seconds in 10ms quanta
    QUANTA_NS = 10_000_000
    TOTAL_NS = 2_000_000_000

    current_vtime = 0
    while current_vtime < TOTAL_NS:
        replies = session.get("sim/clock/advance/0", payload=pack_clock_advance(QUANTA_NS))
        for reply in replies:
            if reply.ok:
                current_vtime, _, _ = unpack_clock_ready(reply.ok.payload.to_bytes())
        # print(f"[TimeAuthority] vtime: {current_vtime} ns")
        # No sleep here, we want to advance as fast as QEMU allows

    print("[TimeAuthority] Reached target virtual time.")
    session.close()

if __name__ == "__main__":
    main()
