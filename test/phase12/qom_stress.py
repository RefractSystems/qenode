#!/usr/bin/env python3
import socket
import json
import sys
import time

def qmp_cmd(sock, cmd, args=None):
    req = {"execute": cmd}
    if args:
        req["arguments"] = args
    sock.sendall((json.dumps(req) + '\n').encode('utf-8'))
    while True:
        resp = json.loads(sock.recv(4096).decode('utf-8').split('\n')[0])
        if "return" in resp or "error" in resp:
            return resp

def main():
    if len(sys.argv) < 2:
        print("Usage: qom_stress.py <qmp_socket_path>")
        sys.exit(1)
        
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(sys.argv[1])
    
    # Wait for greeting and send qmp_capabilities
    sock.recv(4096)
    qmp_cmd(sock, "qmp_capabilities")
    
    print("Starting QOM stress...")
    start_time = time.time()
    i = 0
    while time.time() - start_time < 3: # run for 3 seconds
        obj_id = f"obj_{i}"
        # Create a secret object
        resp = qmp_cmd(sock, "object-add", {"qom-type": "secret", "id": obj_id, "data": "dummy"})
        if "error" in resp:
            print(f"Error adding object: {resp['error']}")
            sys.exit(1)
        # Delete it immediately
        resp = qmp_cmd(sock, "object-del", {"id": obj_id})
        if "error" in resp:
            print(f"Error deleting object: {resp['error']}")
            sys.exit(1)
        i += 1
    print(f"Stress test complete. Performed {i} add/del cycles.")

if __name__ == "__main__":
    main()
