import socket
import struct

VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1

MMIO_REQ_READ = 0
MMIO_REQ_WRITE = 1

SYSC_MSG_RESP = 0
SYSC_MSG_IRQ_SET = 1
SYSC_MSG_IRQ_CLEAR = 2

class MMIOClient:
    def __init__(self, socket_path):
        self.socket_path = socket_path
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.irqs = {}

    def connect(self):
        self.sock.connect(self.socket_path)
        # Handshake
        hs_out = struct.pack("<II", VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION)
        self.sock.sendall(hs_out)
        hs_in = self.sock.recv(8)
        if len(hs_in) < 8:
            raise Exception("Failed to read handshake")
        magic, version = struct.unpack("<II", hs_in)
        if magic != VIRTMCU_PROTO_MAGIC or version != VIRTMCU_PROTO_VERSION:
            raise Exception(f"Handshake failed: {hex(magic)}, {version}")

    def _read_msg(self):
        # sysc_msg is 16 bytes: type(4), irq_num(4), data(8)
        data = b""
        while len(data) < 16:
            chunk = self.sock.recv(16 - len(data))
            if not chunk:
                raise EOFError("Socket closed")
            data += chunk
        return struct.unpack("<IIQ", data)

    def write(self, addr, data, size=4, vtime_ns=0):
        # struct mmio_req { uint8_t type; uint8_t size; uint16_t res1; uint32_t res2; uint64_t vtime_ns; uint64_t addr; uint64_t data; }
        req = struct.pack("<BBHIQQQ", MMIO_REQ_WRITE, size, 0, 0, vtime_ns, addr, data)
        self.sock.sendall(req)

        # Wait for response, but handle IRQs in between
        while True:
            msg_type, irq_num, msg_data = self._read_msg()
            if msg_type == SYSC_MSG_RESP:
                return
            elif msg_type == SYSC_MSG_IRQ_SET:
                self.irqs[irq_num] = True
            elif msg_type == SYSC_MSG_IRQ_CLEAR:
                self.irqs[irq_num] = False

    def read(self, addr, size=4, vtime_ns=0):
        req = struct.pack("<BBHIQQQ", MMIO_REQ_READ, size, 0, 0, vtime_ns, addr, 0)
        self.sock.sendall(req)

        while True:
            msg_type, irq_num, msg_data = self._read_msg()
            if msg_type == SYSC_MSG_RESP:
                return msg_data
            elif msg_type == SYSC_MSG_IRQ_SET:
                self.irqs[irq_num] = True
            elif msg_type == SYSC_MSG_IRQ_CLEAR:
                self.irqs[irq_num] = False

    def close(self):
        self.sock.close()
