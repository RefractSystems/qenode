import struct
import sys
import time

import vproto
import zenoh


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
        print("FAIL: No frame received")
        sys.exit(1)

    vtime, _size = struct.unpack("<QI", rx_frames[0][:12])
    print(f"Original vtime: {orig_vtime}")
    print(f"Forwarded vtime: {vtime}")

    if vtime < orig_vtime:
        print("FAIL: VTime wrapped around!")
        sys.exit(1)
    else:
        print("PASS: VTime did not wrap around.")

    s.close()


if __name__ == "__main__":
    main()
