#!/usr/bin/env python3
"""
test_proto.py — standalone protocol test for the mmio-socket-bridge wire format.

This test starts the SystemC adapter directly, then acts as a fake QEMU client,
sending crafted mmio_req messages over the Unix socket and asserting correct
mmio_resp replies.  No QEMU binary is needed.

Wire protocol (from hw/misc/virtmcu_proto.h):
    mmio_req  = struct { u8 type, u8 size, u16 res1, u32 res2, u64 addr, u64 data }  # 24 bytes
    mmio_resp = struct { u64 data }  # 8 bytes

Usage:
    python3 test/phase5/test_proto.py <adapter_binary>
"""

import struct
import socket
import subprocess
import sys
import os
import time
import signal
import tempfile

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
TOOLS_DIR = os.path.join(os.path.dirname(os.path.dirname(SCRIPT_DIR)), "tools")
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

from vproto import MmioReq, SyscMsg, VirtmcuHandshake, SIZE_MMIO_REQ, SIZE_SYSC_MSG, SIZE_VIRTMCU_HANDSHAKE, VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION, MMIO_REQ_READ, MMIO_REQ_WRITE, SYSC_MSG_RESP, SYSC_MSG_IRQ_SET, SYSC_MSG_IRQ_CLEAR

def send_req(sock, req_type, size, addr, data=0):
    """Send one mmio_req and return the resp.data field."""
    req = MmioReq(type=req_type, size=size, reserved1=0, reserved2=0, vtime_ns=0, addr=addr, data=data)
    sock.sendall(req.pack())
    
    while True:
        resp = b""
        while len(resp) < SIZE_SYSC_MSG:
            chunk = sock.recv(SIZE_SYSC_MSG - len(resp))
            if not chunk:
                raise EOFError("adapter closed connection unexpectedly")
            resp += chunk
        msg = SyscMsg.unpack(resp)
        
        if msg.type == SYSC_MSG_RESP:
            return msg.data
        # Ignore async IRQ messages during sync tests

def wait_for_socket(path, timeout=5.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if os.path.exists(path):
            return True
        time.sleep(0.05)
    return False


def run_tests(adapter_bin):
    sock_path = tempfile.mktemp(suffix=".sock", prefix="virtmcu-proto-test-")
    proc = subprocess.Popen(
        [adapter_bin, sock_path],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    try:
        if not wait_for_socket(sock_path):
            proc.terminate()
            out, err = proc.communicate(timeout=2)
            raise RuntimeError(
                f"adapter socket {sock_path} never appeared.\n"
                f"stdout: {out.decode()}\nstderr: {err.decode()}"
            )

        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(sock_path)
            s.settimeout(5.0)

            # Handshake
            hs_out = VirtmcuHandshake(magic=VIRTMCU_PROTO_MAGIC, version=VIRTMCU_PROTO_VERSION)
            s.sendall(hs_out.pack())
            hs_in_data = s.recv(SIZE_VIRTMCU_HANDSHAKE)
            if len(hs_in_data) != SIZE_VIRTMCU_HANDSHAKE:
                print(f"Handshake failed, got {len(hs_in_data)} bytes")
                return False

            failures = []

            # ── T1: write a value, read it back ──────────────────────────────
            send_req(s, MMIO_REQ_WRITE, 4, addr=0, data=0xdeadbeef)
            got = send_req(s, MMIO_REQ_READ, 4, addr=0)
            if got != 0xdeadbeef:
                failures.append(f"T1 FAIL: wrote 0xdeadbeef, read back 0x{got:08x}")
            else:
                print("T1 PASS: write/read round-trip")

            # ── T2: write to a different register, verify independence ────────
            send_req(s, MMIO_REQ_WRITE, 4, addr=4, data=0x12345678)
            got0 = send_req(s, MMIO_REQ_READ, 4, addr=0)
            got1 = send_req(s, MMIO_REQ_READ, 4, addr=4)
            if got0 != 0xdeadbeef:
                failures.append(f"T2 FAIL: reg0 changed after reg1 write: 0x{got0:08x}")
            elif got1 != 0x12345678:
                failures.append(f"T2 FAIL: reg1 readback wrong: 0x{got1:08x}")
            else:
                print("T2 PASS: register independence")

            # ── T3: overwrite and verify new value ────────────────────────────
            send_req(s, MMIO_REQ_WRITE, 4, addr=0, data=0x00000001)
            got = send_req(s, MMIO_REQ_READ, 4, addr=0)
            if got != 0x00000001:
                failures.append(f"T3 FAIL: expected 0x1, got 0x{got:08x}")
            else:
                print("T3 PASS: overwrite")

            # ── T4: zero write ────────────────────────────────────────────────
            send_req(s, MMIO_REQ_WRITE, 4, addr=0, data=0x0)
            got = send_req(s, MMIO_REQ_READ, 4, addr=0)
            if got != 0:
                failures.append(f"T4 FAIL: expected 0, got 0x{got:08x}")
            else:
                print("T4 PASS: zero write")

            # ── T5: last valid register (index 255) ───────────────────────────
            send_req(s, MMIO_REQ_WRITE, 4, addr=255*4, data=0xfeedface)
            got = send_req(s, MMIO_REQ_READ, 4, addr=255*4)
            if got != 0xfeedface:
                failures.append(f"T5 FAIL: last reg readback wrong: 0x{got:08x}")
            else:
                print("T5 PASS: last register")

            # ── T7: Asynchronous IRQ test ─────────────────────────────────────
            print("T7: Testing asynchronous IRQ...")
            # Writing non-zero to reg 255 should trigger IRQ SET
            # We use sock.sendall directly because send_req expects a RESP
            req = MmioReq(type=MMIO_REQ_WRITE, size=4, reserved1=0, reserved2=0, vtime_ns=0, addr=255*4, data=1)
            s.sendall(req.pack())
            
            irq_set_received = False
            resp_received = False
            deadline = time.time() + 2.0
            while time.time() < deadline and (not irq_set_received or not resp_received):
                chunk = s.recv(SIZE_SYSC_MSG)
                if not chunk: break
                msg = SyscMsg.unpack(chunk)
                if msg.type == SYSC_MSG_IRQ_SET and msg.irq_num == 0:
                    irq_set_received = True
                    print("T7: Received IRQ_SET(0)")
                elif msg.type == SYSC_MSG_RESP:
                    resp_received = True
            
            if not irq_set_received:
                failures.append("T7 FAIL: did not receive IRQ_SET(0) after writing to reg 255")
            elif not resp_received:
                failures.append("T7 FAIL: did not receive RESP after IRQ write")
            else:
                print("T7 PASS: Asynchronous IRQ SET")

            # Writing zero to reg 255 should trigger IRQ CLEAR
            req = MmioReq(type=MMIO_REQ_WRITE, size=4, reserved1=0, reserved2=0, vtime_ns=0, addr=255*4, data=0)
            s.sendall(req.pack())
            irq_clear_received = False
            resp_received = False
            while time.time() < deadline and (not irq_clear_received or not resp_received):
                chunk = s.recv(SIZE_SYSC_MSG)
                if not chunk: break
                msg = SyscMsg.unpack(chunk)
                if msg.type == SYSC_MSG_IRQ_CLEAR and msg.irq_num == 0:
                    irq_clear_received = True
                    print("T7: Received IRQ_CLEAR(0)")
                elif msg.type == SYSC_MSG_RESP:
                    resp_received = True
            
            if not irq_clear_received:
                failures.append("T7 FAIL: did not receive IRQ_CLEAR(0)")
            else:
                print("T7 PASS: Asynchronous IRQ CLEAR")

            # ── T6: throughput / latency benchmark ────────────────────────────
            N = 1000
            t0 = time.monotonic()
            for i in range(N):
                send_req(s, MMIO_REQ_WRITE, 4, addr=0, data=i)
            t1 = time.monotonic()
            elapsed = t1 - t0
            us_per_op = (elapsed / N) * 1e6
            print(f"T6 BENCH: {N} writes in {elapsed*1000:.1f} ms "
                  f"({us_per_op:.1f} µs/op)")
            if us_per_op > 1000:
                failures.append(f"T6 WARN: {us_per_op:.0f} µs/op exceeds 1 ms threshold "
                                f"— socket latency regression?")

        if failures:
            print("\nFAILURES:")
            for f in failures:
                print(" ", f)
            return False
        else:
            print("\nAll protocol tests PASSED")
            return True

    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
        try:
            os.unlink(sock_path)
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <adapter_binary>")
        sys.exit(1)
    ok = run_tests(sys.argv[1])
    sys.exit(0 if ok else 1)
