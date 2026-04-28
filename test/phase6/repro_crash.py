import os
import time

import vproto
import zenoh


def main():
    conf = zenoh.Config()

    router = os.environ.get("ZENOH_ROUTER")
    if router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{router}"]')

    s = zenoh.open(conf)
    pub = s.declare_publisher("sim/eth/frame/malicious/tx")

    print("Sending malformed packet (too short)...")
    pub.put(b"\x00\x01\x02")  # Only 3 bytes, header expects 12

    time.sleep(1)

    # If coordinator is still alive, this should work
    print("Sending valid packet to check if coordinator is alive...")
    pub_valid = s.declare_publisher("sim/eth/frame/1/tx")
    s.declare_subscriber("sim/eth/frame/2/rx", lambda s: print("Received valid packet"))  # noqa: ARG005

    # Node 2 must be "known"
    pub2 = s.declare_publisher("sim/eth/frame/2/tx")
    pub2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    time.sleep(0.5)

    pub_valid.put(vproto.ZenohFrameHeader(1000, 0, 4).pack() + b"ABCD")

    time.sleep(1)
    print("Test finished. Check coordinator logs.")
    s.close()


if __name__ == "__main__":
    main()
