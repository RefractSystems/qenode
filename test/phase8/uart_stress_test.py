import struct
import sys
import threading
import time

import zenoh

# 10 Mbps = 1,250,000 bytes per second
# Interval between bytes = 1 / 1,250,000 = 800 ns
BAUD_10MBPS_INTERVAL_NS = 800
TOTAL_BYTES = 50000
NODE_ID = "0"
TOPIC_BASE = "sim/chardev"

def pack_clock_advance(delta_ns, mujoco_time_ns=0):
    return struct.pack("<QQ", delta_ns, mujoco_time_ns)

def unpack_clock_ready(data):
    # current_vtime_ns (Q), n_frames (I), error_code (I)
    return struct.unpack("<QII", data)

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
    if len(data) >= 12:
        # Skip header
        payload = data[12:]
        if len(received_bytes) == 0 and len(payload) > 0:
            print(f"[UART Stress] First bytes received: {payload}")
        received_bytes.extend(payload)

        if len(received_bytes) >= TOTAL_BYTES:
            received_all_event.set()

sub = session.declare_subscriber(f"{TOPIC_BASE}/{NODE_ID}/tx", on_tx_sample)

pub = session.declare_publisher(f"{TOPIC_BASE}/{NODE_ID}/rx")

print("[UART Stress] Waiting 2s for Zenoh discovery...")
time.sleep(2)

print(f"[UART Stress] Pre-publishing {TOTAL_BYTES} bytes at 10Mbps equivalent...")

# Start at 10ms virtual time — avoids burning 1 billion tight-loop icount instructions
# before first byte arrives (50k bytes at 800ns each = 40ms, so all delivered by ~50ms)
start_vtime = 10_000_000

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

print("[UART Stress] Pre-publish complete. Starting Time Authority to advance clock...")

def time_authority_loop():
    current_vtime = 0
    QUANTA_NS = 10_000_000  # 10ms
    # 50k bytes at 800ns spacing = 40ms; start_vtime = 10ms → all bytes by ~50ms
    TOTAL_NS = 200_000_000  # 200ms — ample margin

    while current_vtime < TOTAL_NS and not received_all_event.is_set():
        replies = session.get("sim/clock/advance/0", payload=pack_clock_advance(QUANTA_NS))
        for reply in replies:
            if reply.ok:
                current_vtime, _, _ = unpack_clock_ready(reply.ok.payload.to_bytes())
        # print(f"[TimeAuthority] vtime: {current_vtime} ns")

t1 = threading.Thread(target=time_authority_loop)
t1.start()

# Wait for all bytes to be echoed back (with 60s timeout for 50k Zenoh put round-trips)
if received_all_event.wait(timeout=60):
    print(f"[UART Stress] Received all {len(received_bytes)} bytes back!")
    if all(b == ord('A') for b in received_bytes[:TOTAL_BYTES]):
        print("[UART Stress] Data integrity verified.")
        session.close()
        sys.exit(0)
    else:
        print("[UART Stress] Data corruption detected!")
        session.close()
        sys.exit(1)
else:
    print(f"[UART Stress] Timeout! Received only {len(received_bytes)} bytes.")
    session.close()
    sys.exit(1)
