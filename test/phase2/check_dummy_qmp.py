#!/usr/bin/env python3
# ==============================================================================
# check_dummy_qmp.py
#
# Connects to QEMU's QMP socket and recursively searches the QOM tree for the
# dynamic `dummy-device`.
# ==============================================================================

import json
import socket
import sys
import time


def check_dummy():
    """
    Main function to connect to QEMU and verify the dummy-device presence.
    """
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)

    # Try to connect to QEMU's QMP socket (wait for QEMU to start)
    # This loop handles the race condition where QEMU is still initializing its socket.
    for _ in range(20):
        try:
            s.connect("qmp.sock")
            break
        except Exception:
            time.sleep(1.0)
    else:
        print("FAILED: Could not connect to QEMU QMP socket (timeout 20s)")
        sys.exit(1)

    # Read the initial QMP greeting message
    s.recv(1024)
    # Negotiate capabilities (required by QMP protocol to enter operational mode)
    s.send(b'{"execute": "qmp_capabilities"}\n')
    s.recv(1024)

    visited = set()

    def find_dummy(path):
        """
        Recursively searches the QOM tree for 'dummy-device'.
        """
        if path in visited:
            return False
        visited.add(path)

        # Request a list of objects at the current path
        req = json.dumps({"execute": "qom-list", "arguments": {"path": path}})
        s.send(req.encode() + b"\n")

        # Read the full response (ending in a newline)
        data = b""
        while b"\n" not in data:
            data += s.recv(4096)

        resp = json.loads(data.decode().strip())
        if "return" not in resp:
            return False

        for item in resp["return"]:
            # Check if this item is our target type
            if item["type"] == "link<dummy-device>" or item["type"] == "child<dummy-device>":
                return True
            # If it's a container (child), recurse into it
            if item["type"].startswith("child<"):
                next_path = path + "/" + item["name"] if path != "/" else "/" + item["name"]
                if find_dummy(next_path):
                    return True
        return False

    # Start the recursive search from the root of the QOM tree
    if find_dummy("/"):
        print("PASSED: 'dummy-device' found in QOM tree!")
        sys.exit(0)
    else:
        print("FAILED: 'dummy-device' NOT found in QOM tree!")
        sys.exit(1)


if __name__ == "__main__":
    check_dummy()
