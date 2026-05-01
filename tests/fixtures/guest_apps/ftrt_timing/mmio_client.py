"""
SOTA Test Module: mmio_client

Context:
This module implements tests for the mmio_client subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of mmio_client.
"""

import socket

import vproto

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
        self.sock.settimeout(2.0)
        self.sock.connect(self.socket_path)
        # Handshake
        hs_out = vproto.VirtmcuHandshake(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION).pack()
        self.sock.sendall(hs_out)
        hs_in = self.sock.recv(vproto.SIZE_VIRTMCU_HANDSHAKE)
        if len(hs_in) < vproto.SIZE_VIRTMCU_HANDSHAKE:
            raise Exception("Failed to read handshake")
        hs = vproto.VirtmcuHandshake.unpack(hs_in)
        if hs.magic != VIRTMCU_PROTO_MAGIC or hs.version != VIRTMCU_PROTO_VERSION:
            raise Exception(f"Handshake failed: {hex(hs.magic)}, {hs.version}")

    def _read_msg(self):
        # sysc_msg is 16 bytes: type(4), irq_num(4), data(8)
        data = b""
        while len(data) < vproto.SIZE_SYSC_MSG:
            chunk = self.sock.recv(vproto.SIZE_SYSC_MSG - len(data))
            if not chunk:
                raise EOFError("Socket closed")
            data += chunk
        msg = vproto.SyscMsg.unpack(data)
        return msg.type, msg.irq_num, msg.data

    def write(self, addr, data, size=4, vtime_ns=0):
        # struct mmio_req { uint8_t type; uint8_t size; uint16_t res1; uint32_t res2; uint64_t vtime_ns; uint64_t addr; uint64_t data; }
        req = vproto.MmioReq(MMIO_REQ_WRITE, size, 0, 0, vtime_ns, addr, data).pack()
        self.sock.sendall(req)

        # Wait for response, but handle IRQs in between
        while True:
            msg_type, irq_num, _msg_data = self._read_msg()
            if msg_type == SYSC_MSG_RESP:
                return
            if msg_type == SYSC_MSG_IRQ_SET:
                self.irqs[irq_num] = True
            elif msg_type == SYSC_MSG_IRQ_CLEAR:
                self.irqs[irq_num] = False

    def read(self, addr, size=4, vtime_ns=0):
        req = vproto.MmioReq(MMIO_REQ_READ, size, 0, 0, vtime_ns, addr, 0).pack()
        self.sock.sendall(req)

        while True:
            msg_type, irq_num, msg_data = self._read_msg()
            if msg_type == SYSC_MSG_RESP:
                return msg_data
            if msg_type == SYSC_MSG_IRQ_SET:
                self.irqs[irq_num] = True
            elif msg_type == SYSC_MSG_IRQ_CLEAR:
                self.irqs[irq_num] = False

    def close(self):
        self.sock.close()
