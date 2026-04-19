import struct
import time

import zenoh


def main():
    conf = zenoh.Config()
    s = zenoh.open(conf)
    pub = s.declare_publisher("sim/eth/frame/malicious/tx")

    print("Sending malformed packet (too short)...")
    pub.put(b"\x00\x01\x02") # Only 3 bytes, header expects 12

    time.sleep(1)

    # If coordinator is still alive, this should work
    print("Sending valid packet to check if coordinator is alive...")
    pub_valid = s.declare_publisher("sim/eth/frame/1/tx")
    s.declare_subscriber("sim/eth/frame/2/rx", lambda s: print("Received valid packet"))

    # Node 2 must be "known"
    pub2 = s.declare_publisher("sim/eth/frame/2/tx")
    pub2.put(struct.pack("<QI", 0, 0))
    time.sleep(0.5)

    pub_valid.put(struct.pack("<QI", 1000, 4) + b"ABCD")

    time.sleep(1)
    print("Test finished. Check coordinator logs.")
    s.close()

if __name__ == "__main__":
    main()
