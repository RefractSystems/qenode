import socket
import struct
import os

# REQ_FMT  = "<BBHIqqq"   # type, size, reserved1, reserved2, vtime_ns, addr, data  (32 bytes)
# (from virtmcu_proto.h: struct mmio_req)
# struct mmio_req {
#     uint8_t type;
#     uint8_t size;
#     uint16_t reserved1;
#     uint32_t reserved2;
#     int64_t vtime_ns;
#     uint64_t addr;
#     uint64_t data;
# };
REQ_FMT = "<BBHIqQQ"
REQ_SIZE = struct.calcsize(REQ_FMT)

RESP_FMT = "<IIQ" # type, irq_num, data
RESP_SIZE = struct.calcsize(RESP_FMT)

def start_server(sock_path):
    if os.path.exists(sock_path):
        os.unlink(sock_path)
    
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(sock_path)
    server.listen(1)
    print(f"Server listening on {sock_path}")
    
    conn, _ = server.accept()
    print("Connected")
    
    while True:
        data = conn.recv(REQ_SIZE)
        if not data:
            break
        
        req_type, size, res1, res2, vtime, addr, val = struct.unpack(REQ_FMT, data)
        print(f"REQ: type={req_type}, size={size}, vtime={vtime}, addr=0x{addr:x}, data=0x{val:x}", flush=True)
        
        # Send response
        resp = struct.pack(RESP_FMT, 0, 0, 0)
        conn.sendall(resp)
    conn.close()
    server.close()

if __name__ == "__main__":
    start_server("/tmp/mmio.sock")
