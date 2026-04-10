#!/usr/bin/env python3
# ==============================================================================
# qmp_probe.py
#
# A developer utility to interactively inspect a running QEMU instance via the
# QEMU Machine Protocol (QMP).
#
# QMP allows for JSON-based control and inspection of QEMU. This tool focus on
# the QEMU Object Model (QOM), which is the internal hierarchical database
# where QEMU stores all its devices, memory regions, and buses.
#
# This tool is essential for:
#   1. Verifying that Device Trees (.dtb) correctly instantiated devices.
#   2. Confirming that dynamic plugins (.so) were auto-loaded and initialized.
#   3. Inspecting real-time state (like registers or memory region sizes).
#
# Usage examples:
#   # Start QEMU with a QMP socket first:
#   ./scripts/run.sh --dtb test/phase1/minimal.dtb -qmp unix:qmp.sock,server,nowait
#
#   # Then in another terminal:
#   python3 tools/qmp_probe.py tree             # Visualize the entire object hierarchy
#   python3 tools/qmp_probe.py list /machine     # List immediate children/properties of /machine
#   python3 tools/qmp_probe.py get /memory size  # Fetch the value of a specific property
# ==============================================================================

import argparse
import json
import socket
import sys


class QMPClient:
    """
    A minimal synchronous QMP client for scriptable inspection.
    """
    def __init__(self, socket_path="qmp.sock"):
        self.socket_path = socket_path
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.buffer = b""

    def connect(self):
        """
        Connects to the QMP socket and performs the initial negotiation.
        """
        try:
            self.sock.connect(self.socket_path)
        except FileNotFoundError:
            print(f"Error: QMP socket '{self.socket_path}' not found.")
            print("Hint: Start QEMU with '-qmp unix:qmp.sock,server,nowait'")
            sys.exit(1)
        except ConnectionRefusedError:
            print(f"Error: Connection refused to '{self.socket_path}'.")
            print("Hint: Is QEMU still running?")
            sys.exit(1)

        # QMP server sends a greeting on connection
        greeting = self._recv_msg()
        if not greeting or "QMP" not in greeting:
            print("Error: Did not receive a valid QMP greeting.")
            sys.exit(1)

        # Capabilities negotiation is mandatory before sending commands
        # We send an empty set of capabilities to enter command mode.
        self.execute("qmp_capabilities")

    def _recv_msg(self):
        """
        Reads one complete JSON message from the socket.
        """
        while b'\n' not in self.buffer:
            data = self.sock.recv(4096)
            if not data:
                return None
            self.buffer += data
        line, self.buffer = self.buffer.split(b'\n', 1)
        return json.loads(line.decode('utf-8'))

    def execute(self, cmd, args=None):
        """
        Executes a QMP command and returns the JSON response.
        """
        req = {"execute": cmd}
        if args:
            req["arguments"] = args

        # QMP commands are JSON objects followed by a newline
        self.sock.send(json.dumps(req).encode('utf-8') + b'\n')

        # Wait for the response (which is also a single JSON object on one line)
        return self._recv_msg()

def dump_tree(client, path="/", depth=0, visited=None):
    """
    Recursively traverses the QOM tree and prints it in a human-readable format.

    Similar to the 'info qom-tree' command in the QEMU monitor.
    """
    if visited is None:
        visited = set()
    if path in visited:
        return
    visited.add(path)

    # qom-list returns children (child<...>) and links (link<...>)
    resp = client.execute("qom-list", {"path": path})
    if 'return' not in resp:
        return

    for item in resp['return']:
        # Print with indentation to show hierarchy
        print("  " * depth + f"{item['name']} ({item['type']})")

        # If it's a child object, recurse into it
        if item['type'].startswith('child<'):
            # Construct the absolute path to the child
            next_path = path + '/' + item['name'] if path != '/' else '/' + item['name']
            dump_tree(client, next_path, depth + 1, visited)

def main():
    parser = argparse.ArgumentParser(
        description="virtmcu QMP Probing Utility",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""Examples:
  %(prog)s tree
  %(prog)s list /machine/unattached
  %(prog)s get /machine/peripheral-anon/device[0] realized
"""
    )
    parser.add_argument(
        "--socket",
        default="qmp.sock",
        help="Path to the QMP Unix socket (default: qmp.sock)"
    )

    subparsers = parser.add_subparsers(dest="command", required=True, help="Sub-commands")

    # 'tree' command: Recursive visualization
    subparsers.add_parser("tree", help="Recursively dump the entire QOM tree")

    # 'list' command: Single-level inspection
    list_parser = subparsers.add_parser("list", help="List properties/children of a specific QOM path")
    list_parser.add_argument("path", help="Absolute QOM path (e.g., /machine)")

    # 'get' command: Fetch a value
    get_parser = subparsers.add_parser("get", help="Get the value of a specific QOM property")
    get_parser.add_argument("path", help="Absolute QOM path to the object")
    get_parser.add_argument("property", help="Name of the property to read")

    args = parser.parse_args()

    # Initialize and connect the client
    client = QMPClient(args.socket)
    client.connect()

    if args.command == "tree":
        print(f"--- QOM Tree (Source: {args.socket}) ---")
        dump_tree(client)

    elif args.command == "list":
        resp = client.execute("qom-list", {"path": args.path})
        if 'return' in resp:
            # Print a simple list of names and types
            for item in resp['return']:
                print(f"{item['name']:<30} ({item['type']})")
        else:
            print(f"Error: {resp.get('error', resp)}")

    elif args.command == "get":
        resp = client.execute("qom-get", {"path": args.path, "property": args.property})
        if 'return' in resp:
            # Pretty-print the JSON value
            print(json.dumps(resp['return'], indent=2))
        else:
            print(f"Error: {resp.get('error', resp)}")

if __name__ == "__main__":
    main()
