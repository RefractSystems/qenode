import struct
import sys
import threading
import time

import zenoh

# 1 Mbps = 125,000 bytes per second
# Interval between bytes = 1 / 125,000 = 8000 ns
BAUD_10MBPS_INTERVAL_NS = 8000
TOTAL_BYTES = 50000
NODE_ID = "0"
TOPIC_BASE = "sim/chardev"

router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

print(f"[UART Stress] Connected to Zenoh router at {router}")

received_bytes = bytearray()
received_all_event = threading.Event()

def on_tx_sample(sample):
    data = sample.payload.to_bytes()
    # print(f"[UART Stress] Received {len(data)} bytes from Zenoh")
    if len(data) >= 12:
        # Skip header
        payload = data[12:]
        received_bytes.extend(payload)
        if len(received_bytes) % 1000 == 0:
             print(f"[UART Stress] Progress: {len(received_bytes)} bytes received")
        if len(received_bytes) >= TOTAL_BYTES:
            received_all_event.set()

sub = session.declare_subscriber(f"{TOPIC_BASE}/{NODE_ID}/tx", on_tx_sample)

def run_test():
    pub = session.declare_publisher(f"{TOPIC_BASE}/{NODE_ID}/rx")

    # Wait for QEMU to initialize fully
    time.sleep(2)

    print(f"[UART Stress] Sending {TOTAL_BYTES} bytes at 10Mbps equivalent...")
    start_vtime = 1_000_000_000 # Start at 1s virtual time to avoid startup noise

    # We send in chunks to not overwhelm Zenoh's internal buffers
    CHUNK_SIZE = 1000
    for i in range(0, TOTAL_BYTES, CHUNK_SIZE):
        chunk_end = min(i + CHUNK_SIZE, TOTAL_BYTES)
        for j in range(i, chunk_end):
            vtime = start_vtime + (j * BAUD_10MBPS_INTERVAL_NS)
            # Header: 8 bytes vtime, 4 bytes size
            header = struct.pack("<QI", vtime, 1)
            payload = header + b"A" # Sending 'A's
            pub.put(payload)
        time.sleep(0.01) # Small sleep to throttle host-side publication

    print("[UART Stress] Send complete. Waiting for echo...")

t1 = threading.Thread(target=run_test)
t1.start()

# Wait for all bytes to be echoed back (with 20s timeout)
if received_all_event.wait(timeout=20):
    print(f"[UART Stress] Received all {len(received_bytes)} bytes back!")
    if all(b == ord('A') for b in received_bytes[:TOTAL_BYTES]):
        print("[UART Stress] Data integrity verified.")
        sys.exit(0)
    else:
        print("[UART Stress] Data corruption detected!")
        sys.exit(1)
else:
    print(f"[UART Stress] Timeout! Received only {len(received_bytes)} bytes.")
    sys.exit(1)

session.close()
