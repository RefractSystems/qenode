#!/usr/bin/env python3
"""
scripts/get-free-port.py

Dynamically finds and returns an available, ephemeral port and/or a test IP.
Used by test suites to avoid hardcoding IP/ports, supporting parallel execution.
"""

import argparse
import os
import socket
import sys
import time
from pathlib import Path

RESERVATION_DIR = Path(
    os.environ.get(
        "VIRTMCU_PORT_RESERVATION_DIR",
        f"/tmp/virtmcu_port_reservations_{os.getuid()}", # virtmcu-allow: absolute_path reasoning="Legacy script"
    )
)


def get_free_port() -> int:
    """
    Finds a free port available on all interfaces and reserves it.
    Uses a file-based registry in /tmp to avoid collisions in parallel tests.
    Avoids the OS ephemeral port range (typically 32768+) to prevent TOCTOU
    conflicts where the OS assigns the released port to an outgoing connection.
    """
    RESERVATION_DIR.mkdir(parents=True, exist_ok=True)
    import secrets

    # Try to find and reserve a port outside the ephemeral range
    for _ in range(1000):
        port = 10000 + secrets.randbelow(22000)
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            try:
                s.bind(("", port))
            except OSError:
                continue

        res_path = RESERVATION_DIR / str(port)
        try:
            # Atomic creation of a reservation file
            res_path.touch(exist_ok=False)
            return port
        except FileExistsError:
            # Port was recently reserved. Check if it's stale (older than 60s)
            try:
                stale_timeout_sec = 60
                if time.time() - res_path.stat().st_mtime > stale_timeout_sec:
                    try:
                        res_path.unlink(missing_ok=True)
                    except OSError:
                        pass  # Someone else cleaned it up
            except OSError:
                pass
            continue

    # Fallback if we can't reserve after many attempts
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        return s.getsockname()[1]


def get_test_ip() -> str:
    """
    Returns a suitable IP for testing.
    Prioritizes:
    1. TEST_IP environment variable.
    2. Loopback 127.0.0.1 as default (most robust for inter-process in container).
    """
    if "TEST_IP" in os.environ:
        return os.environ["TEST_IP"]

    return "127.0.0.1"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Get a free port and/or test IP.")
    parser.add_argument("--ip", action="store_true", help="Return test IP only")
    parser.add_argument("--port", action="store_true", help="Return free port only")
    parser.add_argument("--endpoint", action="store_true", help="Return IP:PORT")
    parser.add_argument("--proto", type=str, default="", help="Prefix with protocol (e.g., tcp/)")

    # Handle the case where no arguments are provided (traditional behavior)
    if len(sys.argv) == 1:
        sys.stdout.write(str(get_free_port()) + "\n")
        sys.exit(0)

    args = parser.parse_args()

    ip = get_test_ip()
    port = get_free_port()

    if args.endpoint:
        sys.stdout.write(f"{args.proto}{ip}:{port}\n")
    elif args.ip:
        sys.stdout.write(f"{ip}\n")
    elif args.port:
        sys.stdout.write(f"{port}\n")
    else:
        # If no specific type requested but flags were provided, default to port
        sys.stdout.write(f"{port}\n")
