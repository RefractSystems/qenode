"""
SOTA Test Module: uart_stress_test
"""

import asyncio
import logging
import sys
import threading
import typing

import zenoh

from tools import vproto
from tools.testing.utils import mock_execution_delay
from tools.testing.virtmcu_test_suite.conftest_core import wait_for_zenoh_discovery

logger = logging.getLogger(__name__)

BAUD_1MBPS_INTERVAL_NS = 10000
TOTAL_BYTES = 50000
NODE_ID = "0"
TOPIC_BASE = "virtmcu/uart"

TEST_BYTE = b"X"
TEST_BYTE_VAL = ord("X")
START_VTIME_NS = 10_000_000
CHUNK_SIZE = 1000
CHUNK_SLEEP_S = 0.01
QUANTUM_NS = 10_000_000
CLOCK_TOTAL_NS = 100_000_000_000
RX_TIMEOUT_S = 300


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

_lock = threading.Lock()
_x_count = 0
_first_logged = False
received_all_event = threading.Event()
_welcome_received = threading.Event()


def on_tx_sample(sample: zenoh.Sample) -> None:
    global _x_count, _first_logged
    raw = sample.payload.to_bytes()
    if len(raw) < vproto.SIZE_ZENOH_FRAME_HEADER:
        return
    payload = raw[vproto.SIZE_ZENOH_FRAME_HEADER :]
    if not payload:
        return

    if b"Interactive UART Echo Ready" in payload:
        logger.info("[UART Stress] Firmware welcome message detected.")
        _welcome_received.set()

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

logger.info("[UART Stress] Waiting for QEMU and Zenoh discovery...")
asyncio.run(wait_for_zenoh_discovery(session, "sim/clock/advance/0"))

if not _welcome_received.wait(timeout=10.0):
    logger.warning("[UART Stress] Timeout waiting for welcome message, proceeding anyway...")

logger.info(f"[UART Stress] Pre-publishing {TOTAL_BYTES} bytes at 1 Mbps equivalent...")

for i in range(0, TOTAL_BYTES, CHUNK_SIZE):
    chunk_end = min(i + CHUNK_SIZE, TOTAL_BYTES)
    for j in range(i, chunk_end):
        vtime = START_VTIME_NS + (j * BAUD_1MBPS_INTERVAL_NS)
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
    typing.cast(typing.Any, session).close()

    sys.exit(0)
else:
    with _lock:
        final_count = _x_count
    logger.info(f"[UART Stress] FAIL: timeout after {RX_TIMEOUT_S} s — received {final_count}/{TOTAL_BYTES} echo bytes")
    typing.cast(typing.Any, session).close()

    sys.exit(1)
