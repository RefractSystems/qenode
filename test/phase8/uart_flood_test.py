import threading
import time

import zenoh

router = "tcp/127.0.0.1:7447"
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

print("[UART Flood] Connected to Zenoh.")


def publish_chardev():
    pub = session.declare_publisher("virtmcu/uart/0/rx")

    # 50,000 bytes blasted at once (far exceeding 32-byte PL011 FIFO)
    # The expected result is hardware-accurate byte dropping, but NO CRASH in QEMU.
    payload = b"X" * 50000

    print(f"[UART Flood] Blasting {len(payload)} bytes into UART RX...")
    pub.put(payload)

    print("[UART Flood] Blast complete. Awaiting crash or stability...")
    time.sleep(2)


t1 = threading.Thread(target=publish_chardev)
t1.start()
t1.join()

print("[UART Flood] Test completed.")
session.close()
