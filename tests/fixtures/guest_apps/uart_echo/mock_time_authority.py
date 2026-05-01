"""
SOTA Test Module: mock_time_authority

Context:
This module implements tests for the mock_time_authority subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of mock_time_authority.
"""

import logging
import sys

import vproto
import zenoh

logger = logging.getLogger(__name__)


# Standard virtmcu ClockAdvanceReq/ClockReadyResp packing
def pack_clock_advance(delta_ns, mujoco_time_ns=0, quantum_number=0):
    return vproto.ClockAdvanceReq(delta_ns, mujoco_time_ns, quantum_number).pack()


def unpack_clock_ready(data):
    resp = vproto.ClockReadyResp.unpack(data)
    return resp.current_vtime_ns, resp.n_frames, resp.error_code, resp.quantum_number


def main():
    if len(sys.argv) <= 1:
        logger.error(f"Usage: {sys.argv[0]} <router_endpoint>")
        sys.exit(1)
    router = sys.argv[1]
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    session = zenoh.open(conf)

    logger.info("[TimeAuthority] Advancing clock on sim/clock/advance/0...")

    # Advance 2 seconds in 10ms quanta
    QUANTA_NS = 10_000_000  # noqa: N806
    TOTAL_NS = 2_000_000_000  # noqa: N806

    current_vtime = 0
    q_num = 0
    while current_vtime < TOTAL_NS:
        replies = session.get("sim/clock/advance/0", payload=pack_clock_advance(QUANTA_NS, quantum_number=q_num))
        for reply in replies:
            if reply.ok:
                current_vtime, _, _, _ = unpack_clock_ready(reply.ok.payload.to_bytes())
        q_num += 1
        # logger.info(f"[TimeAuthority] vtime: {current_vtime} ns")
        # No sleep here, we want to advance as fast as QEMU allows

    logger.info("[TimeAuthority] Reached target virtual time.")
    session.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
