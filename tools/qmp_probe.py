#!/usr/bin/env python3
# ==============================================================================
# qmp_probe.py
#
# A developer utility to interactively inspect a running QEMU instance via QMP.
# It connects to QEMU's QMP Unix socket and can recursively dump the QOM 
# (QEMU Object Model) tree or query specific device properties.
#
# This is invaluable for verifying that dynamic modules or DTB nodes were
# instantiated correctly without having to drop into GDB or the QEMU monitor.
#
# Usage:
#   python3 tools/qmp_probe.py tree          # Dumps the entire QOM tree
#   python3 tools/qmp_probe.py get <path> <prop> # Gets a specific property
# ==============================================================================

import socket
import json
import argparse
import sys

class QMPClient:
    def __init__(self, socket_path="qmp.sock"):
        self.socket_path = socket_path
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.buffer = b""

    def connect(self):
        try:
            self.sock.connect(self.socket_path)
        except FileNotFoundError:
            print(f"Error: QMP socket '{self.socket_path}' not found.")
            print("Ensure QEMU is running with '-qmp unix:qmp.sock,server,nowait'")
            sys.exit(1)
        except ConnectionRefusedError:
            print(f"Error: Connection refused to '{self.socket_path}'.")
            sys.exit(1)

        # Read greeting
        self._recv_msg()
        # Negotiate capabilities
        self.execute("qmp_capabilities")

    def _recv_msg(self):
        while b'\n' not in self.buffer:
            data = self.sock.recv(4096)
            if not data:
                return None
            self.buffer += data
        line, self.buffer = self.buffer.split(b'\n', 1)
        return json.loads(line.decode('utf-8'))

    def execute(self, cmd, args=None):
        req = {"execute": cmd}
        if args:
            req["arguments"] = args
        self.sock.send(json.dumps(req).encode('utf-8') + b'\n')
        return self._recv_msg()

def dump_tree(client, path="/", depth=0, visited=None):
    """Recursively traverses and prints the QOM tree."""
    if visited is None:
        visited = set()
    if path in visited:
        return
    visited.add(path)

    resp = client.execute("qom-list", {"path": path})
    if 'return' not in resp:
        return

    for item in resp['return']:
        print("  " * depth + f"{item['name']} ({item['type']})")
        if item['type'].startswith('child<'):
            next_path = path + '/' + item['name'] if path != '/' else '/' + item['name']
            dump_tree(client, next_path, depth + 1, visited)

def main():
    parser = argparse.ArgumentParser(description="QEMU QMP Probe Utility")
    parser.add_argument("--socket", default="qmp.sock", help="Path to QMP socket (default: qmp.sock)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    # 'tree' command
    subparsers.add_parser("tree", help="Recursively dump the QOM tree")

    # 'get' command
    get_parser = subparsers.add_parser("get", help="Get a specific QOM property")
    get_parser.add_argument("path", help="QOM object path (e.g., /machine/peripheral-anon/device[0])")
    get_parser.add_argument("property", help="Property name (e.g., size, realized)")

    args = parser.parse_args()

    client = QMPClient(args.socket)
    client.connect()

    if args.command == "tree":
        print("--- QOM Tree ---")
        dump_tree(client)
    elif args.command == "get":
        resp = client.execute("qom-get", {"path": args.path, "property": args.property})
        if 'return' in resp:
            print(json.dumps(resp['return'], indent=2))
        else:
            print(f"Error: {resp.get('error', resp)}")

if __name__ == "__main__":
    main()
