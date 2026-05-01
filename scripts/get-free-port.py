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


def get_free_port():
    """Finds a free port available on all interfaces."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        return s.getsockname()[1]

def get_test_ip():
    """
    Returns a suitable IP for testing.
    Prioritizes:
    1. TEST_IP environment variable.
    2. Primary interface IP (to avoid hardcoding 127.0.0.1).
    3. Loopback 127.0.0.1 as fallback.
    """
    if "TEST_IP" in os.environ:
        return os.environ["TEST_IP"]

    # Try to get the primary interface IP that has external connectivity
    try:
        # We don't need a real connection, just to see what IP would be used
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            # 8.8.8.8 is just a placeholder, no traffic is sent
            s.connect(("8.8.8.8", 80))
            return s.getsockname()[0]
    except Exception:
        # Fallback to a simple hostname lookup
        try:
            return socket.gethostbyname(socket.gethostname())
        except Exception:
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
