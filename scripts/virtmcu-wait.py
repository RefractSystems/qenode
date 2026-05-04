#!/usr/bin/env python3
"""
VirtMCU Transport Readiness Tool

This tool ensures that the Zenoh transport layer and the simulation coordinator
are fully ready before dependent services (like QEMU or physics engines) start.
It uses Zenoh liveliness probes to verify routing table propagation and queries
 the coordinator's ready probe.

Usage:
    python3 virtmcu-wait.py [--router <endpoint>] [--coordinator] [--timeout <seconds>]
"""

import argparse
import asyncio
import logging
import os
import sys
from typing import Any, cast

import zenoh

# Setup basic logging to stderr
logging.basicConfig(level=logging.INFO, format='%(levelname)s: %(message)s')
logger = logging.getLogger("virtmcu-wait")


async def ensure_session_routing(session: zenoh.Session, timeout: float = 5.0) -> None:
    """
    Block until the router has propagated this session's declarations.
    Uses the Zenoh Liveliness API for a deterministic roundtrip.
    """
    probe_topic = f"sim/test/probe/{os.getpid()}/{id(session):x}"
    logger.info("Establishing routing barrier on %s...", probe_topic)

    token = await asyncio.to_thread(lambda: session.liveliness().declare_token(probe_topic))
    try:
        # Wait until we can see our own token
        event = asyncio.Event()
        loop = asyncio.get_running_loop()

        def on_liveliness(sample: zenoh.Sample) -> None:
            if sample.kind == zenoh.SampleKind.PUT:
                loop.call_soon_threadsafe(event.set)

        sub = await asyncio.to_thread(lambda: session.liveliness().declare_subscriber(probe_topic, on_liveliness))
        try:
            # Check if already present
            l_get = await asyncio.to_thread(lambda: list(session.liveliness().get(probe_topic)))
            if any(s.key_expr == probe_topic for s in l_get):
                logger.info("Routing fabric confirmed (immediate).")
                return

            await asyncio.wait_for(event.wait(), timeout=timeout)
            logger.info("Routing fabric confirmed (event).")
        finally:
            await asyncio.to_thread(sub.undeclare)
    finally:
        await asyncio.to_thread(cast(Any, token).undeclare)


async def check_coordinator(session: zenoh.Session, timeout: float = 5.0) -> bool:
    """Query the coordinator's ready probe."""
    logger.info("Checking coordinator readiness...")
    try:
        replies = await asyncio.to_thread(lambda: list(session.get("sim/coordinator/ready_probe", timeout=timeout)))
        if any(r.is_ok for r in replies):
            logger.info("Coordinator is READY.")
            return True
        logger.error("Coordinator did not respond to ready probe.")
        return False
    except (zenoh.Error, RuntimeError) as e:
        logger.error("Failed to query coordinator: %s", e)
        return False


async def main() -> None:
    parser = argparse.ArgumentParser(description="Wait for VirtMCU transport and coordinator readiness.")
    parser.add_argument(
        "--router",
        type=str,
        default=os.environ.get("ZENOH_ROUTER", "tcp/localhost:7447"),
        help="Zenoh router endpoint",
    )
    parser.add_argument("--coordinator", action="store_true", help="Wait for the simulation coordinator to be ready")
    parser.add_argument("--timeout", type=float, default=30.0, help="Total timeout in seconds")
    args = parser.parse_args()

    config = zenoh.Config()
    if args.router:
        config.insert_json5("mode", '"client"')
        config.insert_json5("connect/endpoints", f'["{args.router}"]')
        config.insert_json5("scouting/multicast/enabled", "false")

    try:
        logger.info("Connecting to Zenoh router at %s...", args.router)
        session = await asyncio.to_thread(lambda: zenoh.open(config).wait())

        # 1. Routing Barrier
        await ensure_session_routing(session, timeout=min(10.0, args.timeout))

        # 2. Optional Coordinator Check
        if args.coordinator:
            if not await check_coordinator(session, timeout=min(10.0, args.timeout)):
                sys.exit(1)

        logger.info("VirtMCU infrastructure is READY.")
    except TimeoutError:
        logger.error("Timed out waiting for infrastructure readiness.")
        sys.exit(1)
    except (zenoh.Error, RuntimeError) as e:
        logger.error("An error occurred: %s", e)
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
