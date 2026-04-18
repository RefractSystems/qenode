import socket
import struct
import sys
import os
import time

# From virtmcu_proto.h
VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1

def main():
    if len(sys.argv) < 3:
        print("Usage: malicious_adapter.py <socket_path> <mode>")
        print("Modes: hang, crash")
        sys.exit(1)

    sock_path = sys.argv[1]
    mode = sys.argv[2]
    
    if os.path.exists(sock_path):
        os.remove(sock_path)

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(sock_path)
    server.listen(1)

    print(f"Malicious adapter ({mode}) listening on {sock_path}", flush=True)
    conn, _ = server.accept()
    print("Connection accepted", flush=True)

    # 1. Handshake
    data = conn.recv(8)
    if not data:
        print("No data received for handshake", flush=True)
        return
    magic, version = struct.unpack("<II", data)
    print(f"Received handshake: magic=0x{magic:X}, version={version}", flush=True)
    
    hs_out = struct.pack("<II", VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION)
    conn.sendall(hs_out)
    print("Sent handshake", flush=True)

    # 2. Wait for first MMIO request
    # req_data = conn.recv(32) # mmio_req is 32 bytes
    req_data = b""
    while len(req_data) < 32:
        chunk = conn.recv(32 - len(req_data))
        if not chunk:
            break
        req_data += chunk

    if len(req_data) == 32:
        req_type = req_data[0]
        print(f"Received MMIO request: type={req_type}", flush=True)
        
        if mode == "hang":
            print("Ignoring request to trigger timeout in QEMU...")
            # Keep the connection open but do nothing
            while True:
                try:
                    if not conn.recv(1024):
                        break
                except:
                    break
        elif mode == "crash":
            print("Closing connection immediately to simulate crash...")
            conn.close()
        else:
            print(f"Unknown mode: {mode}")

    conn.close()
    server.close()

if __name__ == "__main__":
    main()
