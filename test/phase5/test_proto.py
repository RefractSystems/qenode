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

# ── Wire format ───────────────────────────────────────────────────────────────
REQ_FMT  = "<BBHIqqq"   # type, size, reserved1, reserved2, addr, data  (24 bytes)
RESP_FMT = "<IIQ"      # type, irq_num, data  (16 bytes)
REQ_SIZE  = struct.calcsize(REQ_FMT)
RESP_SIZE = struct.calcsize(RESP_FMT)
assert REQ_SIZE == 24, f"REQ_SIZE={REQ_SIZE}, expected 24"
assert RESP_SIZE == 16,  f"RESP_SIZE={RESP_SIZE}, expected 16"

MMIO_READ  = 0
MMIO_WRITE = 1

SYSC_MSG_RESP = 0
SYSC_MSG_IRQ_SET = 1
SYSC_MSG_IRQ_CLEAR = 2

def send_req(sock, req_type, size, addr, data=0):
    """Send one mmio_req and return the resp.data field."""
    pkt = struct.pack(REQ_FMT, req_type, size, 0, 0, 0, addr, data)
    sock.sendall(pkt)
    
    while True:
        resp = b""
        while len(resp) < RESP_SIZE:
            chunk = sock.recv(RESP_SIZE - len(resp))
            if not chunk:
                raise EOFError("adapter closed connection unexpectedly")
            resp += chunk
        msg_type, irq_num, value = struct.unpack(RESP_FMT, resp)
        
        if msg_type == SYSC_MSG_RESP:
            return value
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

            failures = []

            # ── T1: write a value, read it back ──────────────────────────────
            send_req(s, MMIO_WRITE, 4, addr=0, data=0xdeadbeef)
            got = send_req(s, MMIO_READ, 4, addr=0)
            if got != 0xdeadbeef:
                failures.append(f"T1 FAIL: wrote 0xdeadbeef, read back 0x{got:08x}")
            else:
                print("T1 PASS: write/read round-trip")

            # ── T2: write to a different register, verify independence ────────
            send_req(s, MMIO_WRITE, 4, addr=4, data=0x12345678)
            got0 = send_req(s, MMIO_READ, 4, addr=0)
            got1 = send_req(s, MMIO_READ, 4, addr=4)
            if got0 != 0xdeadbeef:
                failures.append(f"T2 FAIL: reg0 changed after reg1 write: 0x{got0:08x}")
            elif got1 != 0x12345678:
                failures.append(f"T2 FAIL: reg1 readback wrong: 0x{got1:08x}")
            else:
                print("T2 PASS: register independence")

            # ── T3: overwrite and verify new value ────────────────────────────
            send_req(s, MMIO_WRITE, 4, addr=0, data=0x00000001)
            got = send_req(s, MMIO_READ, 4, addr=0)
            if got != 0x00000001:
                failures.append(f"T3 FAIL: expected 0x1, got 0x{got:08x}")
            else:
                print("T3 PASS: overwrite")

            # ── T4: zero write ────────────────────────────────────────────────
            send_req(s, MMIO_WRITE, 4, addr=0, data=0x0)
            got = send_req(s, MMIO_READ, 4, addr=0)
            if got != 0:
                failures.append(f"T4 FAIL: expected 0, got 0x{got:08x}")
            else:
                print("T4 PASS: zero write")

            # ── T5: last valid register (index 255) ───────────────────────────
            send_req(s, MMIO_WRITE, 4, addr=255*4, data=0xfeedface)
            got = send_req(s, MMIO_READ, 4, addr=255*4)
            if got != 0xfeedface:
                failures.append(f"T5 FAIL: last reg readback wrong: 0x{got:08x}")
            else:
                print("T5 PASS: last register")

            # ── T7: Asynchronous IRQ test ─────────────────────────────────────
            print("T7: Testing asynchronous IRQ...")
            # Writing non-zero to reg 255 should trigger IRQ SET
            # We use sock.sendall directly because send_req expects a RESP
            pkt = struct.pack(REQ_FMT, MMIO_WRITE, 4, 0, 0, 0, 255*4, 1)
            s.sendall(pkt)
            
            irq_set_received = False
            resp_received = False
            deadline = time.time() + 2.0
            while time.time() < deadline and (not irq_set_received or not resp_received):
                chunk = s.recv(RESP_SIZE)
                if not chunk: break
                msg_type, irq_num, value = struct.unpack(RESP_FMT, chunk)
                if msg_type == SYSC_MSG_IRQ_SET and irq_num == 0:
                    irq_set_received = True
                    print("T7: Received IRQ_SET(0)")
                elif msg_type == SYSC_MSG_RESP:
                    resp_received = True
            
            if not irq_set_received:
                failures.append("T7 FAIL: did not receive IRQ_SET(0) after writing to reg 255")
            elif not resp_received:
                failures.append("T7 FAIL: did not receive RESP after IRQ write")
            else:
                print("T7 PASS: Asynchronous IRQ SET")

            # Writing zero to reg 255 should trigger IRQ CLEAR
            pkt = struct.pack(REQ_FMT, MMIO_WRITE, 4, 0, 0, 0, 255*4, 0)
            s.sendall(pkt)
            irq_clear_received = False
            resp_received = False
            while time.time() < deadline and (not irq_clear_received or not resp_received):
                chunk = s.recv(RESP_SIZE)
                if not chunk: break
                msg_type, irq_num, value = struct.unpack(RESP_FMT, chunk)
                if msg_type == SYSC_MSG_IRQ_CLEAR and irq_num == 0:
                    irq_clear_received = True
                    print("T7: Received IRQ_CLEAR(0)")
                elif msg_type == SYSC_MSG_RESP:
                    resp_received = True
            
            if not irq_clear_received:
                failures.append("T7 FAIL: did not receive IRQ_CLEAR(0)")
            else:
                print("T7 PASS: Asynchronous IRQ CLEAR")

            # ── T6: throughput / latency benchmark ────────────────────────────
            N = 1000
            t0 = time.monotonic()
            for i in range(N):
                send_req(s, MMIO_WRITE, 4, addr=0, data=i)
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
