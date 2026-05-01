"""
SOTA Test Module: radio_determinism

Context:
This module implements tests for the radio_determinism subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of radio_determinism.
"""

import logging
import sys
from pathlib import Path

import zenoh

from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)

RF_HEADER_SIZE = 14

session = None
ping_responded = False


def on_sample(sample: zenoh.Sample) -> None:
    global session, ping_responded
    payload = sample.payload.to_bytes()
    if len(payload) < RF_HEADER_SIZE:
        return

    vtime = int.from_bytes(payload[:8], "little")
    size = int.from_bytes(payload[8:12], "little")
    rssi = payload[12]
    lqi = payload[13]
    data = payload[RF_HEADER_SIZE:]

    # 802.15.4 FCF: bits 0-2 are frame type. Type 2 is ACK.
    if size >= 2:
        fcf = int.from_bytes(data[:2], "little")
        if (fcf & 0x07) == 0x02:
            return

    if ping_responded:
        return
    ping_responded = True

    logger.info(f"[{vtime}] Received RF packet: size={size} RSSI={rssi} LQI={lqi}")

    # 1. Respond with WRONG address after 1ms virtual time
    resp1_vtime = vtime + 1000000
    resp1_data = (
        (0x8841).to_bytes(2, "little")
        + (0x02).to_bytes(1, "little")
        + (0xABCD).to_bytes(2, "little")
        + (0x5678).to_bytes(2, "little")
        + (0x1234).to_bytes(2, "little")
        + (0).to_bytes(2, "little")
        + b"MISMATCHED ACK"
    )
    msg1 = (
        resp1_vtime.to_bytes(8, "little")
        + len(resp1_data).to_bytes(4, "little")
        + (0xCE).to_bytes(1, "little")
        + (0xFF).to_bytes(1, "little")
        + resp1_data
    )
    logger.info(f"[{resp1_vtime}] Sending MISMATCHED response...")
    session.put("sim/rf/ieee802154/0/rx", msg1)  # type: ignore[attr-defined]

    # 2. Respond with CORRECT address after 2ms virtual time
    resp2_vtime = vtime + 2000000
    resp2_data = (
        (0x8861).to_bytes(2, "little")
        + (0x03).to_bytes(1, "little")
        + (0xABCD).to_bytes(2, "little")
        + (0x1234).to_bytes(2, "little")
        + (0x5678).to_bytes(2, "little")
        + (0).to_bytes(2, "little")
        + b"MATCHED ACK"
    )
    msg2 = (
        resp2_vtime.to_bytes(8, "little")
        + len(resp2_data).to_bytes(4, "little")
        + (0xCE).to_bytes(1, "little")
        + (0xFF).to_bytes(1, "little")
        + resp2_data
    )
    logger.info(f"[{resp2_vtime}] Sending MATCHED response...")
    session.put("sim/rf/ieee802154/0/rx", msg2)  # type: ignore[attr-defined]


def on_tx_sample(sample: zenoh.Sample) -> None:
    payload = sample.payload.to_bytes()
    if len(payload) < RF_HEADER_SIZE:
        return

    vtime = int.from_bytes(payload[:8], "little")
    size = int.from_bytes(payload[8:12], "little")
    data = payload[RF_HEADER_SIZE:]

    if size == 3 and (data[0] & 0x07) == 0x02:
        logger.info(f"[{vtime}] RECEIVED AUTO-ACK for seq {data[2]}")
        with (Path(__file__).resolve().parent / "ack_received.tmp").open("w") as f:
            f.write("OK")


def main() -> None:
    global session
    node_id = sys.argv[1] if len(sys.argv) > 1 else "0"
    if len(sys.argv) <= 2:
        logger.error(f"Usage: {sys.argv[0]} <router_endpoint>")
        sys.exit(1)
    router = sys.argv[2]

    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    session = zenoh.open(conf)

    sub_topic = f"sim/rf/ieee802154/{node_id}/tx"
    logger.info(f"Listening on {sub_topic}...")
    session.declare_subscriber(sub_topic, on_sample)
    session.declare_subscriber(sub_topic, on_tx_sample)

    try:
        while True:
            mock_execution_delay(1)  # SLEEP_EXCEPTION: keepalive loop
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
