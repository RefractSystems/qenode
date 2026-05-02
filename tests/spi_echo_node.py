"""
SOTA Test Module: spi_echo_node

Context:
This module implements tests for the spi_echo_node subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of spi_echo_node.
"""

from __future__ import annotations

import asyncio
import logging
import sys
import typing

import zenoh

from tools import vproto
from tools.testing.virtmcu_test_suite.topics import SimTopic

logger = logging.getLogger(__name__)


def main() -> None:
    if len(sys.argv) < 2:
        logger.info("Usage: spi_echo_node.py <router_endpoint>")
        sys.exit(1)

    router = sys.argv[1]
    config = zenoh.Config()
    config.insert_json5("connect/endpoints", f'["{router}"]')
    config.insert_json5("scouting/multicast/enabled", "false")

    logger.info(f"Connecting to Zenoh on {router}...")
    session = zenoh.open(config)  # ZENOH_OPEN_EXCEPTION: standalone script executed by test

    topic = SimTopic.spi_base("spi0", 0)

    def on_query(query: zenoh.Query) -> None:
        if query.payload is None:
            return
        payload = query.payload.to_bytes()
        header_size = vproto.SIZE_ZENOH_SPI_HEADER
        if len(payload) >= header_size + 4:
            # Parse header using vproto
            header = vproto.ZenohSPIHeader.unpack(payload[:header_size])
            data = payload[header_size : header_size + 4]
            val = int.from_bytes(data, "little")
            logger.info(f"Received SPI transfer: 0x{val:08x} at vtime {header.delivery_vtime_ns}")
            # Echo back
            query.reply(topic, data)

    logger.info(f"Declaring queryable on {topic}...")
    _ = session.declare_queryable(topic, on_query)

    try:
        # Keepalive loop using asyncio event (better than sleep)
        loop = asyncio.new_event_loop()
        asyncio.set_event_loop(loop)
        loop.run_until_complete(asyncio.Event().wait())
    except KeyboardInterrupt:
        pass
    finally:
        typing.cast(typing.Any, session).close()


if __name__ == "__main__":
    main()
