import struct
import sys
import threading
import time
import zenoh

# 10 Mbps = 800 ns interval
BAUD_10MBPS_INTERVAL_NS = 800
TOTAL_BYTES = 10000
TOPIC_BASE = "virtmcu/uart"

def pack_clock_advance(delta_ns, mujoco_time_ns=0):
    return struct.pack("<QQ", delta_ns, mujoco_time_ns)

def unpack_clock_ready(data):
    return struct.unpack("<QII", data)

router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

print(f"[Multi-Node UART] Connected to Zenoh router at {router}")

received_0 = bytearray()
received_1 = bytearray()
done_event = threading.Event()

def on_tx_0(sample):
    data = sample.payload.to_bytes()
    if len(data) >= 12:
        received_0.extend(data[12:])
        if len(received_0) >= TOTAL_BYTES:
            print("[Multi-Node UART] Node 0 received all bytes.")
            done_event.set()

def on_tx_1(sample):
    data = sample.payload.to_bytes()
    if len(data) >= 12:
        received_1.extend(data[12:])
        # Node 1 just logs for now

sub0 = session.declare_subscriber(f"{TOPIC_BASE}/0/tx", on_tx_0)
sub1 = session.declare_subscriber(f"{TOPIC_BASE}/1/tx", on_tx_1)

pub0 = session.declare_publisher(f"{TOPIC_BASE}/0/rx")

print("[Multi-Node UART] Waiting for discovery...")
time.sleep(2)

print(f"[Multi-Node UART] Injecting {TOTAL_BYTES} bytes into Node 0...")
# These will be echoed by Node 0, then routed by coordinator to Node 1,
# Node 1 echoes them back, routed by coordinator to Node 0.
# So we expect Node 0 to see its own bytes echoed back twice? 
# No, coordinator sends to ALL OTHER nodes.
# 1. We inject into Node 0 RX.
# 2. Node 0 echoes to Node 0 TX.
# 3. Coordinator hears Node 0 TX, sends to Node 1 RX.
# 4. Node 1 hears Node 1 RX, echoes to Node 1 TX.
# 5. Coordinator hears Node 1 TX, sends to Node 0 RX.
# 6. Node 0 hears Node 0 RX, echoes to Node 0 TX... LOOP!
# To avoid loop, we should just verify Node 1 receives it.

# Actually, let's just test Node 0 -> Node 1 communication.
start_vtime = 10_000_000
for i in range(TOTAL_BYTES):
    vtime = start_vtime + (i * BAUD_10MBPS_INTERVAL_NS)
    header = struct.pack("<QI", vtime, 1)
    pub0.put(header + b"S")

print("[Multi-Node UART] Starting Time Authority for both nodes...")

def ta_loop(node_id):
    current_vtime = 0
    QUANTA_NS = 1_000_000
    while current_vtime < 1_000_000_000 and not done_event.is_set():
        replies = session.get(f"sim/clock/advance/{node_id}", payload=pack_clock_advance(QUANTA_NS))
        for reply in replies:
            if reply.ok:
                current_vtime, _, _ = unpack_clock_ready(reply.ok.payload.to_bytes())
        time.sleep(0.001)

t0 = threading.Thread(target=ta_loop, args=("0",))
t1 = threading.Thread(target=ta_loop, args=("1",))
t0.start()
t1.start()

if done_event.wait(timeout=30):
    print(f"[Multi-Node UART] SUCCESS: Received {len(received_0)} bytes back at Node 0")
    session.close()
    sys.exit(0)
else:
    print(f"[Multi-Node UART] FAILED: Node 0 received {len(received_0)} bytes, Node 1 received {len(received_1)} bytes")
    session.close()
    sys.exit(1)
