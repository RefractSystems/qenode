"""
SOTA Test Module: uart_multi_node_stress_test

Context:
This module implements tests for the uart_multi_node_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of uart_multi_node_stress_test.
"""

import logging
import sys
import threading
import typing

import zenoh

from tools import vproto
from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)

# 10 Mbps = 800 ns interval
BAUD_10MBPS_INTERVAL_NS = 800
TOTAL_BYTES = 10000
TOPIC_BASE = "virtmcu/uart"


def pack_clock_advance(delta_ns: int, mujoco_time_ns: int = 0, quantum_number: int = 0) -> bytes:
    return vproto.ClockAdvanceReq(delta_ns, mujoco_time_ns, quantum_number).pack()


def unpack_clock_ready(data: bytes) -> tuple[int, int, int, int]:
    resp = vproto.ClockReadyResp.unpack(data)
    return resp.current_vtime_ns, resp.n_frames, resp.error_code, resp.quantum_number


if len(sys.argv) <= 1:
    sys.exit(1)
router = sys.argv[1]
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

logger.info(f"[Multi-Node UART] Connected to Zenoh router at {router}")

received_0 = bytearray()
received_1 = bytearray()
done_event = threading.Event()


def on_tx_0(sample: zenoh.Sample) -> None:
    data = sample.payload.to_bytes()
    if len(data) >= 12:
        received_0.extend(data[12:])
        if len(received_0) >= TOTAL_BYTES:
            logger.info("[Multi-Node UART] Node 0 received all bytes.")
            done_event.set()


def on_tx_1(sample: zenoh.Sample) -> None:
    data = sample.payload.to_bytes()
    if len(data) >= 12:
        received_1.extend(data[12:])
        # Node 1 just logs for now


sub0 = session.declare_subscriber(f"{TOPIC_BASE}/0/tx", on_tx_0)
sub1 = session.declare_subscriber(f"{TOPIC_BASE}/1/tx", on_tx_1)

pub0 = session.declare_publisher(f"{TOPIC_BASE}/0/rx")

logger.info("[Multi-Node UART] Waiting for discovery...")
mock_execution_delay(2)  # SLEEP_EXCEPTION: mock test simulating execution/spacing

logger.info(f"[Multi-Node UART] Injecting {TOTAL_BYTES} bytes into Node 0...")
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
    header = vproto.ZenohFrameHeader(vtime, 0, 1).pack()
    pub0.put(header + b"S")

logger.info("[Multi-Node UART] Starting Time Authority for both nodes...")


def ta_loop(node_id: int) -> None:
    current_vtime = 0
    QUANTA_NS = 1_000_000  # noqa: N806
    while current_vtime < 1_000_000_000 and not done_event.is_set():
        replies = session.get(f"sim/clock/advance/{node_id}", payload=pack_clock_advance(QUANTA_NS))
        for reply in replies:
            if reply.ok:
                current_vtime, _, _, _ = unpack_clock_ready(reply.ok.payload.to_bytes())
        mock_execution_delay(0.001)  # SLEEP_EXCEPTION: mock test simulating execution/spacing


t0 = threading.Thread(target=ta_loop, args=("0",))
t1 = threading.Thread(target=ta_loop, args=("1",))
t0.start()
t1.start()

if done_event.wait(timeout=30):
    logger.info(f"[Multi-Node UART] SUCCESS: Received {len(received_0)} bytes back at Node 0")
    typing.cast(typing.Any, session).close()
    sys.exit(0)
else:
    logger.info(
        f"[Multi-Node UART] FAILED: Node 0 received {len(received_0)} bytes, Node 1 received {len(received_1)} bytes"
    )
    session.close()  # type: ignore[no-untyped-call]
    sys.exit(1)
