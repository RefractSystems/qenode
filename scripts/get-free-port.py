#!/usr/bin/env python3
"""
scripts/get-free-port.py

Dynamically finds and returns an available, ephemeral port on localhost.
Used by test suites (e.g., conftest.py) to assign unique ports to Zenoh routers
or other services, preventing "Address already in use" conflicts during
parallel test execution (pytest -n auto).
"""

import socket


def get_free_port():
    """Finds a free port on localhost."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        return s.getsockname()[1]


if __name__ == "__main__":
    print(get_free_port())
