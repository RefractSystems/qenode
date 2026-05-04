#!/usr/bin/env python3
"""
zenoh_pcap_dumper.py - Standalone tool to record VirtMCU Zenoh traffic to PCAP.

This is a wrapper around virtmcu_extcap.py, providing a more intuitive CLI
for standalone recording.
"""

import argparse
import asyncio
import logging

from tools.wireshark.virtmcu_extcap import capture_loop

logger = logging.getLogger(__name__)

if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    
    parser = argparse.ArgumentParser(description="VirtMCU Zenoh PCAP Dumper")
    parser.add_argument("-o", "--output", required=True, help="Output PCAP file path (use '-' for stdout)")
    parser.add_argument("-s", "--session", default="tcp/localhost:7447", help="Zenoh router endpoint")
    parser.add_argument("-t", "--topic", default="sim/coord/**/rx", help="Zenoh topic to subscribe to")
    parser.add_argument("--legacy", action="store_true", help="Subscribe to legacy sim/comm/** topics")

    args = parser.parse_args()
    
    topic = args.topic
    if args.legacy and topic == "sim/coord/**/rx":
        topic = "sim/comm/**"

    logger.info("Starting Zenoh PCAP Dumper...")
    logger.info(f"  Session: {args.session}")
    logger.info(f"  Topic:   {topic}")
    logger.info(f"  Output:  {args.output}")
    logger.info("Press Ctrl+C to stop.")

    try:
        asyncio.run(capture_loop(args.output, args.session, topic, args.legacy))
    except KeyboardInterrupt:
        logger.info("\nStopped.")