import struct
import sys
import threading
import time

import zenoh

# 10 Mbps = 1,250,000 bytes per second
# Interval between bytes = 1 / 1,250,000 = 800 ns
BAUD_10MBPS_INTERVAL_NS = 800
TOTAL_BYTES = 50_000
NODE_ID = "0"
TOPIC_BASE = "virtmcu/uart"

# Test byte: 0x58 ('X') does not appear in the firmware welcome message
# ("Interactive UART Echo Ready.\r\nType something: "), so we can safely
# count only 'X' bytes to separate echo bytes from startup noise.
TEST_BYTE = b"X"
TEST_BYTE_VAL = ord("X")

# Start at 10 ms virtual time so QEMU doesn't burn instructions before the
# first byte (50k × 800 ns = 40 ms, all delivered by ~50 ms vtime).  # noqa: RUF003
START_VTIME_NS = 10_000_000

CHUNK_SIZE = 1_000  # bytes per Zenoh publication burst
CHUNK_SLEEP_S = 0.01  # throttle between bursts (avoids overwhelming router)
QUANTUM_NS = 10_000_000  # 10 ms per clock-advance quantum

# Virtual time ceiling: 50k bytes × 10 µs retry per byte (1-byte PL011 FIFO)  # noqa: RUF003
# = 500 ms + 40 ms byte timestamps + 10 ms start offset + margin = 1 s
CLOCK_TOTAL_NS = 1_000_000_000

RX_TIMEOUT_S = 60  # wall-clock timeout waiting for all echoes


def _pack_clock_advance(delta_ns: int, mujoco_time_ns: int = 0) -> bytes:
    return struct.pack("<QQ", delta_ns, mujoco_time_ns)


def _unpack_clock_ready(data: bytes) -> tuple[int, int, int]:
    # current_vtime_ns (Q), n_frames (I), error_code (I)
    return struct.unpack("<QII", data)


def _open_session(router: str) -> zenoh.Session:
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    return zenoh.open(conf)


router = sys.argv[1] if len(sys.argv) > 1 else "tcp/127.0.0.1:7447"
session = _open_session(router)
print(f"[UART Stress] Connected to Zenoh router at {router}")

# Thread-safe echo counter.  We only count TEST_BYTE_VAL ('X') to isolate echo
# bytes from firmware startup noise (welcome message).
_lock = threading.Lock()
_x_count = 0
_first_logged = False
received_all_event = threading.Event()


def on_tx_sample(sample: zenoh.Sample) -> None:
    global _x_count, _first_logged
    raw = sample.payload.to_bytes()
    if len(raw) < 12:
        return
    # Skip 12-byte ZenohFrameHeader (delivery_vtime_ns:u64 + size:u32)
    payload = raw[12:]
    if not payload:
        return

    new_x = sum(1 for b in payload if b == TEST_BYTE_VAL)
    if new_x == 0:
        return

    with _lock:
        if not _first_logged:
            print(f"[UART Stress] First echo bytes received: {bytes(payload[:8])}")
            _first_logged = True
        _x_count += new_x
        if _x_count >= TOTAL_BYTES:
            received_all_event.set()


_sub = session.declare_subscriber(f"{TOPIC_BASE}/{NODE_ID}/tx", on_tx_sample)
_pub = session.declare_publisher(f"{TOPIC_BASE}/{NODE_ID}/rx")

print("[UART Stress] Waiting 2 s for Zenoh discovery...")
time.sleep(2)

print(f"[UART Stress] Pre-publishing {TOTAL_BYTES} bytes at 10 Mbps equivalent...")

for i in range(0, TOTAL_BYTES, CHUNK_SIZE):
    chunk_end = min(i + CHUNK_SIZE, TOTAL_BYTES)
    for j in range(i, chunk_end):
        vtime = START_VTIME_NS + (j * BAUD_10MBPS_INTERVAL_NS)
        # ZenohFrameHeader: delivery_vtime_ns (u64 LE) + size (u32 LE)
        header = struct.pack("<QI", vtime, 1)
        _pub.put(header + TEST_BYTE)
    time.sleep(CHUNK_SLEEP_S)

print("[UART Stress] Pre-publish complete. Starting Time Authority...")


def _time_authority_loop() -> None:
    current_vtime = 0
    while current_vtime < CLOCK_TOTAL_NS and not received_all_event.is_set():
        replies = session.get(
            "sim/clock/advance/0",
            payload=_pack_clock_advance(QUANTUM_NS),
        )
        for reply in replies:
            if reply.ok:
                current_vtime, _, _ = _unpack_clock_ready(reply.ok.payload.to_bytes())


_ta_thread = threading.Thread(target=_time_authority_loop, daemon=True)
_ta_thread.start()

if received_all_event.wait(timeout=RX_TIMEOUT_S):
    with _lock:
        final_count = _x_count

    print(f"[UART Stress] Received {final_count} echo bytes (expected {TOTAL_BYTES})")

    if final_count != TOTAL_BYTES:
        print(f"[UART Stress] FAIL: byte count mismatch ({final_count} != {TOTAL_BYTES})")
        session.close()
        sys.exit(1)

    print("[UART Stress] Data integrity verified.")
    session.close()
    sys.exit(0)
else:
    with _lock:
        final_count = _x_count
    print(f"[UART Stress] FAIL: timeout after {RX_TIMEOUT_S} s — received {final_count}/{TOTAL_BYTES} echo bytes")
    session.close()
    sys.exit(1)
