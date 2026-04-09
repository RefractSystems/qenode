#!/usr/bin/env python3
# ==============================================================================
# check_dummy_qmp.py
#
# Connects to QEMU's QMP socket and recursively searches the QOM tree for the
# dynamic `dummy-device`.
# ==============================================================================

import socket
import json
import time
import sys

def check_dummy():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    
    # Try to connect to QEMU's QMP socket (wait for QEMU to start)
    for _ in range(10):
        try:
            s.connect('qmp.sock')
            break
        except Exception:
            time.sleep(0.5)
    else:
        print("FAILED: Could not connect to QEMU QMP socket")
        sys.exit(1)

    # Read greeting
    s.recv(1024)
    # Negotiate capabilities
    s.send(b'{"execute": "qmp_capabilities"}\n')
    s.recv(1024)

    visited = set()
    def find_dummy(path):
        if path in visited: return False
        visited.add(path)
        
        req = json.dumps({"execute": "qom-list", "arguments": {"path": path}})
        s.send(req.encode() + b'\n')
        
        data = b""
        while b'\n' not in data:
            data += s.recv(4096)
            
        resp = json.loads(data.decode().strip())
        if 'return' not in resp: return False
        
        for item in resp['return']:
            if item['type'] == 'link<dummy-device>' or item['type'] == 'child<dummy-device>':
                return True
            if item['type'].startswith('child<'):
                next_path = path + '/' + item['name'] if path != '/' else '/' + item['name']
                if find_dummy(next_path):
                    return True
        return False

    if find_dummy('/'):
        print("PASSED: 'dummy-device' found in QOM tree!")
        sys.exit(0)
    else:
        print("FAILED: 'dummy-device' NOT found in QOM tree!")
        sys.exit(1)

if __name__ == "__main__":
    check_dummy()
