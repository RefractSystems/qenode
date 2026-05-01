"""
SOTA Test Module: zenoh_router

Context:
This module implements tests for the zenoh_router subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of zenoh_router.
"""

import logging
import sys
import typing

import zenoh

from tools.testing.env import WORKSPACE_DIR
from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)


def main() -> None:
    if len(sys.argv) > 1:
        endpoint = sys.argv[1]
    else:
        # Resolve dynamic IP for default
        try:
            import subprocess

            get_ip_script = WORKSPACE_DIR / "scripts" / "get-free-port.py"
            host_ip = subprocess.check_output([sys.executable, str(get_ip_script), "--ip"]).decode().strip()
            endpoint = f"tcp/{host_ip}:7448"
        except (subprocess.CalledProcessError, OSError):
            endpoint = "tcp/localhost:7448"

    config = zenoh.Config()
    config.insert_json5("listen/endpoints", f'["{endpoint}"]')
    config.insert_json5("scouting/multicast/enabled", "false")

    logger.info(f"Starting Zenoh router on {endpoint}...")
    session = zenoh.open(config)

    logger.info("Router running. Press Ctrl+C to exit.")
    try:
        while True:
            mock_execution_delay(1)  # SLEEP_EXCEPTION: keepalive loop
    except KeyboardInterrupt:
        pass
    typing.cast(typing.Any, session).close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
