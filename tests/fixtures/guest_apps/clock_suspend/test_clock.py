"""
SOTA Test Module: test_clock

Context:
This module implements tests for the test_clock subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock.
"""

import logging
import sys
import typing

import zenoh

from tools.vproto import ClockAdvanceReq, ClockReadyResp

logger = logging.getLogger(__name__)

DELTA1_NS = 1_000_000
DELTA2_NS = 2_000_000
TOPIC = "sim/clock/advance/0"
TIMEOUT_S = 10.0


Q_NUM = 0


def pack_req(delta_ns: int) -> bytes:
    global Q_NUM
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=Q_NUM)
    Q_NUM += 1
    return req.pack()


def unpack_rep(data: bytes) -> int:
    resp = ClockReadyResp.unpack(data)
    if resp.error_code != 0:
        logger.error(f"WARNING: Reply error_code = {resp.error_code} (1=STALL, 2=ZENOH_ERROR)")
    return resp.current_vtime_ns


def send_query(session: zenoh.Session, delta_ns: int, label: str) -> int:
    replies = list(session.get(TOPIC, payload=pack_req(delta_ns), timeout=TIMEOUT_S))
    if not replies:
        logger.error(f"{label}: TIMEOUT — no reply received")
        sys.exit(1)
    reply = replies[0]
    if getattr(reply, "err", None) is not None:
        logger.error(f"{label}: ERROR reply: {reply.err}")
        sys.exit(1)
    if not hasattr(reply, "ok"):
        logger.error(f"{label}: NO 'ok' in reply: {reply}")
        sys.exit(1)
    if reply.ok is None:
        logger.error(f"{label}: reply.ok IS NONE. Full reply: {reply}")
        sys.exit(1)
    return unpack_rep(reply.ok.payload.to_bytes())


def main() -> None:
    if len(sys.argv) <= 1:
        logger.error(f"Usage: {sys.argv[0]} <router_endpoint>")
        sys.exit(1)
    router = sys.argv[1]
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(config)

    vtime1 = send_query(session, DELTA1_NS, "Q1")
    logger.info(f"Q1 vtime = {vtime1} ns")

    vtime2 = send_query(session, DELTA2_NS, "Q2")
    logger.info(f"Q2 vtime = {vtime2} ns  (target approx {vtime1 + DELTA1_NS})")
    if vtime2 < vtime1:
        logger.error(f"FAIL: Q2 vtime {vtime2} < Q1 vtime {vtime1}")
        sys.exit(1)

    vtime3 = send_query(session, 1_000_000, "Q3")
    logger.info(f"Q3 vtime = {vtime3} ns  (target approx {vtime2 + DELTA2_NS})")

    if vtime3 < vtime2 + DELTA2_NS:
        logger.error(f"FAIL: Q3 vtime {vtime3} < Q2 vtime {vtime2} + DELTA2 {DELTA2_NS}")
        sys.exit(1)

    typing.cast(typing.Any, session).close()
    logger.info("PASS")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
