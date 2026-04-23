import struct
import sys
import threading
import time

import zenoh

router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"
config = zenoh.Config()
config.insert_json5("mode", '"client"')
config.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(config)

print("[Flood] Connected to Zenoh.")


def publish_netdev():
    pub = session.declare_publisher("sim/network/0/tx")

    # 12 byte header (8 byte vtime, 4 byte size)
    header = struct.pack("<QI", 0, 10)
    payload = header + b"1234567890"

    print("[Flood] Blasting 50,000 packets rapidly to trigger backpressure/OOM...")

    # Blast packets
    for _i in range(50000):
        pub.put(payload)

    print("[Flood] Blast complete. Awaiting crash or stability...")
    time.sleep(2)


t1 = threading.Thread(target=publish_netdev)
t1.start()
t1.join()

print("[Flood] Test completed.")
session.close()
