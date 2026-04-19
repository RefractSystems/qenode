import threading
import time

import zenoh

router = "tcp/127.0.0.1:7447"
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

print("[Stress] Connected to Zenoh.")

def publish_chardev():
    pub = session.declare_publisher("virtmcu/uart/0/rx")
    for i in range(1000):
        # 12 byte header (8 byte vtime, 4 byte size) + payload
        import struct
        header = struct.pack("<QI", 0, 5)
        payload = header + b"Hello"
        pub.put(payload)
        time.sleep(0.001)

def publish_ui():
    pub = session.declare_publisher("sim/ui/0/button/1")
    for i in range(1000):
        pub.put(b"\x01" if i % 2 == 0 else b"\x00")
        time.sleep(0.001)

t1 = threading.Thread(target=publish_chardev)
t2 = threading.Thread(target=publish_ui)

t1.start()
t2.start()

t1.join()
t2.join()

print("[Stress] Finished publishing 2000 events.")
session.close()
