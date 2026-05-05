"""
SOTA Test Module: radio_determinism

Context:
This module implements tests for the radio_determinism subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of radio_determinism.
"""

import logging
import sys

import flatbuffers.util
import zenoh

from tools import vproto
from tools.testing.utils import mock_execution_delay
from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

logger = logging.getLogger(__name__)

session: zenoh.Session | None = None
ping_responded = False


def on_sample(sample: zenoh.Sample) -> None:
    global session, ping_responded
    try:
        payload = sample.payload.to_bytes()
        logger.debug(f"Received sample on {sample.key_expr}, payload len={len(payload)}")

        if len(payload) < 4:
            return

        size_prefix = flatbuffers.util.GetSizePrefix(payload, 0)
        fb_len = 4 + size_prefix
        if len(payload) < fb_len:
            return

        header = vproto.Rf802154Header.unpack(payload)
        vtime = header.delivery_vtime_ns
        size = header.size
        rssi = header.rssi
        lqi = header.lqi
        data = payload[fb_len:]

        logger.info(f"[{vtime}] Received RF packet: size={size} RSSI={rssi} LQI={lqi}")

        # 802.15.4 FCF: bits 0-2 are frame type. Type 2 is ACK.
        if size >= 2:
            fcf = int.from_bytes(data[:2], "little")  # LINT_EXCEPTION: int_from_bytes
            if (fcf & 0x07) == 0x02:
                logger.info("Ignoring ACK frame.")
                return

        if b"PING" in data:
            logger.info(f"[{vtime}] Ping detected! Responding...")
            resp_vtime = vtime + 5000000  # 5ms later
            resp_data = b"PONG"

            hdr = vproto.Rf802154Header(resp_vtime, 0, len(resp_data), -50, 0xFF).pack()
            msg = hdr + resp_data

            sub_topic = sample.key_expr
            rx_topic = str(sub_topic).replace("/tx", "/rx")

            if session:
                session.put(rx_topic, msg)
                logger.info(f"[{resp_vtime}] Sent PONG to {rx_topic}")

            # Write a marker file to verify we responded
            with open("ack_received.tmp", "w") as f:
                f.write("OK")
    except Exception as e:  # noqa: BLE001
        logger.error(f"ERROR in on_sample: {e}")


def on_tx_sample(sample: zenoh.Sample) -> None:
    try:
        payload = sample.payload.to_bytes()
        if len(payload) < 4:
            return

        header = vproto.Rf802154Header.unpack(payload)
        vtime = header.delivery_vtime_ns
        size = header.size

        size_prefix = flatbuffers.util.GetSizePrefix(payload, 0)
        fb_len = 4 + size_prefix
        data = payload[fb_len:]

        if b"Radio test packet" in data:
            logger.info(f"[{vtime}] Received Radio test packet! size={size}")
            resp1_vtime = vtime + 10000000  # 10ms later
            resp1_data = b"MATCHED ACK"
            hdr1 = vproto.Rf802154Header(resp1_vtime, 0, len(resp1_data), -40, 0xFF).pack()
            msg1 = hdr1 + resp1_data

            rx_topic = str(sample.key_expr).replace("/tx", "/rx")
            if session:
                session.put(rx_topic, msg1)
                logger.info(f"[{resp1_vtime}] Sent MATCHED response...")

            # Also send a mismatched one that should be filtered
            resp2_vtime = vtime + 20000000  # 20ms later
            resp2_data = b"MISMATCHED ACK"
            hdr2 = vproto.Rf802154Header(resp2_vtime, 0, len(resp2_data), -30, 0xFF).pack()
            msg2 = hdr2 + resp2_data
            if session:
                session.put(rx_topic, msg2)
                logger.info(f"[{resp2_vtime}] Sent MISMATCHED response...")
    except Exception as e:  # noqa: BLE001
        logger.error(f"ERROR in on_tx_sample: {e}")


def main() -> None:
    global session
    if len(sys.argv) <= 2:
        logger.error(f"Usage: {sys.argv[0]} <node_id> <router_endpoint>")
        sys.exit(1)

    node_id = sys.argv[1]
    router = sys.argv[2]

    logger.info(f"Connecting to Zenoh router at {router}...")
    session = open_client_session(connect=router)
    logger.info("Connected to Zenoh.")

    sub_topic = f"sim/rf/ieee802154/{node_id}/tx"
    logger.info(f"Listening on {sub_topic}...")
    session.declare_subscriber(sub_topic, on_sample)
    session.declare_subscriber(sub_topic, on_tx_sample)

    # Debug: listen on everything
    def on_any(sample: zenoh.Sample) -> None:
        logger.debug(f"ANY: {sample.key_expr} ({len(sample.payload)})")

    session.declare_subscriber("**", on_any)

    try:
        while True:
            mock_execution_delay(1)  # SLEEP_EXCEPTION: keepalive loop
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    logging.basicConfig(level=logging.DEBUG, format="%(message)s")
    main()
