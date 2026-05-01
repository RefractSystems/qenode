"""
SOTA Test Module: uart_flood_test

Context:
This module implements tests for the uart_flood_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of uart_flood_test.
"""

import logging
import sys
import threading
import time

import zenoh

logger = logging.getLogger(__name__)

if len(sys.argv) <= 1:
    sys.exit(1)
    router = sys.argv[1]
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", f'["{router}"]')
session = zenoh.open(conf)

logger.info("[UART Flood] Connected to Zenoh.")


def publish_chardev():
    pub = session.declare_publisher("virtmcu/uart/0/rx")

    # 50,000 bytes blasted at once (far exceeding 32-byte PL011 FIFO)
    # The expected result is hardware-accurate byte dropping, but NO CRASH in QEMU.
    payload = b"X" * 50000

    logger.info(f"[UART Flood] Blasting {len(payload)} bytes into UART RX...")
    pub.put(payload)

    logger.info("[UART Flood] Blast complete. Awaiting crash or stability...")
    time.sleep(2)


t1 = threading.Thread(target=publish_chardev)
t1.start()
t1.join()

logger.info("[UART Flood] Test completed.")
session.close()
