"""
telemetry_listener.py - Zenoh-based telemetry trace viewer.

Subscribes to simulation telemetry topics via Zenoh and prints formatted
trace events (CPU state changes, IRQs, peripheral events) to the console.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import zenoh

if TYPE_CHECKING:
    from zenoh import Sample


logger = logging.getLogger(__name__)


def on_sample(sample: Sample) -> None:
    """Callback for incoming telemetry samples."""
    from Virtmcu.Telemetry.TraceEvent import TraceEvent

    payload = sample.payload.to_bytes()
    try:
        ev = TraceEvent.GetRootAs(payload, 0)

        ts = ev.TimestampNs()
        ev_type = ev.Type()
        ev_id = ev.Id()
        val = ev.Value()
        name = ev.DeviceName()
        name_str = name.decode("utf-8") if name else ""

        if ev_type == 0:  # CPU_STATE
            type_str = "CPU_STATE"
            id_str = f"cpu={ev_id}"
        elif ev_type == 1:  # IRQ
            type_str = "IRQ"
            slot = ev_id >> 16
            pin = ev_id & 0xFFFF
            id_str = f"slot={slot:2} pin={pin:2}"
            if name_str:
                id_str += f" ({name_str})"
        elif ev_type == 2:  # PERIPHERAL
            type_str = "PERIPHERAL"
            id_str = f"id={ev_id}"
        else:
            type_str = "UNKNOWN"
            id_str = f"id={ev_id}"

        logger.info(f"[{ts:15}] {type_str:10} {id_str} val={val:3}")
    except (ValueError, TypeError, IndexError) as e:
        logger.warning(f"Received malformed payload of size {len(payload)}: {payload.hex()} ({e})")


def main() -> None:
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(description="Zenoh Telemetry Listener")
    parser.add_argument("node_id", nargs="?", default="0", help="Node ID to listen for")
    parser.add_argument("--router", help="Zenoh router endpoint")
    args = parser.parse_args()

    topic = f"sim/telemetry/trace/{args.node_id}"
    logger.info(f"Listening on {topic}...")

    conf = zenoh.Config()
    if args.router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{args.router}"]')

    session = zenoh.open(conf)
    _sub = session.declare_subscriber(topic, on_sample)

    import asyncio
    import contextlib

    async def wait_forever() -> None:
        await asyncio.Event().wait()

    with contextlib.suppress(KeyboardInterrupt):
        asyncio.run(wait_forever())


if __name__ == "__main__":
    main()
