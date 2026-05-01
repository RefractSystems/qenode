"""
SOTA Test Module: qom_stress

Context:
This module implements tests for the qom_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of qom_stress.
"""

#!/usr/bin/env python3
import json
import logging
import socket
import sys
import time

logger = logging.getLogger(__name__)


def qmp_cmd(sock, cmd, args=None):
    req = {"execute": cmd}
    if args:
        req["arguments"] = args
    sock.sendall((json.dumps(req) + "\n").encode("utf-8"))
    while True:
        resp = json.loads(sock.recv(4096).decode("utf-8").split("\n")[0])
        if "return" in resp or "error" in resp:
            return resp


def main():
    if len(sys.argv) < 2:
        logger.info("Usage: qom_stress.py <qmp_socket_path>")
        sys.exit(1)

    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(sys.argv[1])

    # Wait for greeting and send qmp_capabilities
    sock.recv(4096)
    qmp_cmd(sock, "qmp_capabilities")

    logger.info("Starting QOM stress...")
    start_time = time.time()
    i = 0
    while time.time() - start_time < 3:  # run for 3 seconds
        obj_id = f"obj_{i}"
        # Create a secret object
        resp = qmp_cmd(sock, "object-add", {"qom-type": "secret", "id": obj_id, "data": "dummy"})
        if "error" in resp:
            logger.error(f"Error adding object: {resp['error']}")
            sys.exit(1)
        # Delete it immediately
        resp = qmp_cmd(sock, "object-del", {"id": obj_id})
        if "error" in resp:
            logger.error(f"Error deleting object: {resp['error']}")
            sys.exit(1)
        i += 1
    logger.info(f"Stress test complete. Performed {i} add/del cycles.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
