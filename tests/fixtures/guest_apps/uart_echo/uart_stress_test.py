"""
SOTA Test Module: uart_stress_test

Context:
This module implements tests for the uart_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of uart_stress_test.
"""

import logging
import sys
import threading
import typing

import zenoh

from tools import vproto
from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)

# 1 Mbps = 100,000 bytes per second (approx)
# Interval between bytes = 1 / 100,000 = 10,000 ns
BAUD_1MBPS_INTERVAL_NS = 10000
TOTAL_BYTES = 50000
NODE_ID = "0"
TOPIC_BASE = "virtmcu/uart"

# Test byte: 0x58 ('X') does not appear in the firmware welcome message
# ("Interactive UART Echo Ready.\r\nType something: "), so we can safely
# count only 'X' bytes to separate echo bytes from startup noise.
TEST_BYTE = b"X"
TEST_BYTE_VAL = ord("X")

# Start at 10 ms virtual time
START_VTIME_NS = 10_000_000

CHUNK_SIZE = 1000  # bytes per Zenoh publication burst
CHUNK_SLEEP_S = 0.01  # throttle between bursts (avoids overwhelming router)
QUANTUM_NS = 10_000_000  # 10 ms per clock-advance quantum

# Virtual time ceiling: 100 seconds (plenty of room)
CLOCK_TOTAL_NS = 100_000_000_000
RX_TIMEOUT_S = 300  # wall-clock timeout waiting for all echoes


def _pack_clock_advance(delta_ns: int, mujoco_time_ns: int = 0, quantum_number: int = 0) -> bytes:
    return vproto.ClockAdvanceReq(delta_ns, mujoco_time_ns, quantum_number).pack()


def _unpack_clock_ready(data: bytes) -> tuple[int, int, int, int]:
    resp = vproto.ClockReadyResp.unpack(data)
    return resp.current_vtime_ns, resp.n_frames, resp.error_code, resp.quantum_number


def _open_session(router: str) -> zenoh.Session:
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    return zenoh.open(conf)


if len(sys.argv) <= 1:
    sys.exit(1)
router = sys.argv[1]
session = _open_session(router)
logger.info(f"[UART Stress] Connected to Zenoh router at {router}")

# Thread-safe echo counter.  We only count TEST_BYTE_VAL ('X') to isolate echo
# bytes from firmware startup noise (welcome message).
_lock = threading.Lock()
_x_count = 0
_first_logged = False
received_all_event = threading.Event()


def on_tx_sample(sample: zenoh.Sample) -> None:
    global _x_count, _first_logged
    raw = sample.payload.to_bytes()
    if len(raw) < vproto.SIZE_ZENOH_FRAME_HEADER:
        return
    # Skip ZenohFrameHeader
    payload = raw[vproto.SIZE_ZENOH_FRAME_HEADER :]
    if not payload:
        return

    new_x = sum(1 for b in payload if b == TEST_BYTE_VAL)
    if new_x == 0:
        return

    with _lock:
        if not _first_logged:
            logger.info(f"[UART Stress] First echo bytes received: {bytes(payload[:8]).hex()}")
            _first_logged = True
        _x_count += new_x
        if _x_count >= TOTAL_BYTES:
            received_all_event.set()


_sub = session.declare_subscriber(f"{TOPIC_BASE}/{NODE_ID}/tx", on_tx_sample)
_pub = session.declare_publisher(f"{TOPIC_BASE}/{NODE_ID}/rx")

logger.info("[UART Stress] Waiting 5 s for QEMU and Zenoh discovery...")
mock_execution_delay(5)  # SLEEP_EXCEPTION: ensure QEMU is fully up and firmware welcome message is sent

logger.info(f"[UART Stress] Pre-publishing {TOTAL_BYTES} bytes at 1 Mbps equivalent...")

for i in range(0, TOTAL_BYTES, CHUNK_SIZE):
    chunk_end = min(i + CHUNK_SIZE, TOTAL_BYTES)
    for j in range(i, chunk_end):
        vtime = START_VTIME_NS + (j * BAUD_1MBPS_INTERVAL_NS)
        # ZenohFrameHeader: delivery_vtime_ns, sequence_number, size
        # Using sequence number 0 for pre-published bytes as they have distinct vtimes.
        header = vproto.ZenohFrameHeader(vtime, 0, 1).pack()
        _pub.put(header + TEST_BYTE)
    mock_execution_delay(CHUNK_SLEEP_S)  # SLEEP_EXCEPTION: mock test simulating execution/spacing

logger.info("[UART Stress] Pre-publish complete. Starting Time Authority...")


def _time_authority_loop() -> None:
    current_vtime = 0
    quantum_number = 0
    while current_vtime < CLOCK_TOTAL_NS and not received_all_event.is_set():
        quantum_number += 1
        replies = session.get(
            "sim/clock/advance/0",
            payload=_pack_clock_advance(QUANTUM_NS, 0, quantum_number),
        )
        for reply in replies:
            if reply.ok:
                vtime, _, error_code, _ = _unpack_clock_ready(reply.ok.payload.to_bytes())
                if error_code == 0:
                    current_vtime = vtime
                else:
                    logger.warning(f"[UART Stress] Clock stall/error: {error_code}")


_ta_thread = threading.Thread(target=_time_authority_loop, daemon=True)
_ta_thread.start()

if received_all_event.wait(timeout=RX_TIMEOUT_S):
    with _lock:
        final_count = _x_count

    logger.info(f"[UART Stress] Received {final_count} echo bytes (expected {TOTAL_BYTES})")

    if final_count != TOTAL_BYTES:
        logger.info(f"[UART Stress] FAIL: byte count mismatch ({final_count} != {TOTAL_BYTES})")
        typing.cast(typing.Any, session).close()
        sys.exit(1)

    logger.info("[UART Stress] Data integrity verified.")
    session.close()  # type: ignore[no-untyped-call]
    sys.exit(0)
else:
    with _lock:
        final_count = _x_count
    logger.info(f"[UART Stress] FAIL: timeout after {RX_TIMEOUT_S} s — received {final_count}/{TOTAL_BYTES} echo bytes")
    session.close()  # type: ignore[no-untyped-call]
    sys.exit(1)
