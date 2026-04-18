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
        device_id = f"dummy_dev_{i}"
        # Create a dummy device
        qmp_cmd(sock, "device_add", {"driver": "sys-bus-device", "id": device_id})
        # Delete it immediately
        qmp_cmd(sock, "device_del", {"id": device_id})
        i += 1
    print(f"Stress test complete. Performed {i} add/del cycles.")

if __name__ == "__main__":
    main()
