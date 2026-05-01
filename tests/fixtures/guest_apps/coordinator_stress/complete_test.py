"""
SOTA Test Module: complete_test

Context:
This module implements tests for the complete_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of complete_test.
"""

import json
import logging
import os
import queue
import sys
import time
from pathlib import Path

import flatbuffers
import vproto
import zenoh
from virtmcu.rf import RfHeader

sys.path.append(str(Path(__file__).resolve().parent))

logger = logging.getLogger(__name__)


def pack_rf_header(vtime, size, rssi, lqi):
    builder = flatbuffers.Builder(64)
    RfHeader.Start(builder)
    RfHeader.AddDeliveryVtimeNs(builder, vtime)
    RfHeader.AddSequenceNumber(builder, 0)
    RfHeader.AddSize(builder, size)
    RfHeader.AddRssi(builder, rssi)
    RfHeader.AddLqi(builder, lqi)
    hdr = RfHeader.End(builder)
    builder.FinishSizePrefixed(hdr)
    return builder.Output()


def unpack_rf_header(data):
    sz = int.from_bytes(data[:4], "little")
    hdr = RfHeader.RfHeader.GetRootAs(data[4 : 4 + sz], 0)
    return hdr.DeliveryVtimeNs(), hdr.Size(), hdr.Rssi(), hdr.Lqi(), 4 + sz


def main():
    conf = zenoh.Config()

    router = os.environ.get("ZENOH_ROUTER")
    if router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{router}"]')
        conf.insert_json5("scouting/multicast/enabled", "false")

    # Increase task workers to prevent deadlocks when blocking in query handlers.
    import contextlib

    with contextlib.suppress(Exception):
        conf.insert_json5("transport/shared/task_workers", "16")

    s = zenoh.open(conf)

    # Under ASan, the coordinator and network stack are much slower.
    multiplier = 1.0
    if os.environ.get("VIRTMCU_USE_ASAN") == "1":
        multiplier = 10.0

    timeout_val = 5.0 * multiplier
    malformed_timeout = 1.0 * multiplier

    results = {
        "eth": False,
        "uart": False,
        "sysc": False,
        "rf": False,
        "rf_hci": False,
        "rf_sensitivity": False,
        "overflow": False,
        "topology": False,
        "malformed": False,
    }

    # 1. ETH test
    rx_eth = queue.Queue()
    s.declare_subscriber("sim/eth/frame/2/rx", lambda sample: rx_eth.put(sample.payload.to_bytes()))
    pub_eth_tx1 = s.declare_publisher("sim/eth/frame/1/tx")
    pub_eth_tx2 = s.declare_publisher("sim/eth/frame/2/tx")

    # 2. UART test
    rx_uart = queue.Queue()
    s.declare_subscriber("virtmcu/uart/2/rx", lambda sample: rx_uart.put(sample.payload.to_bytes()))
    pub_uart_tx1 = s.declare_publisher("virtmcu/uart/1/tx")
    pub_uart_tx2 = s.declare_publisher("virtmcu/uart/2/tx")

    # 3. SystemC test
    rx_sysc = queue.Queue()
    s.declare_subscriber("sim/systemc/frame/2/rx", lambda sample: rx_sysc.put(sample.payload.to_bytes()))
    pub_sysc_tx1 = s.declare_publisher("sim/systemc/frame/1/tx")
    pub_sysc_tx2 = s.declare_publisher("sim/systemc/frame/2/tx")

    # 4. RF test (802.15.4)
    rx_rf = queue.Queue()
    s.declare_subscriber("sim/rf/ieee802154/1/rx", lambda sample: rx_rf.put(sample.payload.to_bytes()))
    pub_rf_tx0 = s.declare_publisher("sim/rf/ieee802154/0/tx")
    pub_rf_tx1 = s.declare_publisher("sim/rf/ieee802154/1/tx")
    pub_rf_tx2 = s.declare_publisher("sim/rf/ieee802154/2/tx")

    # 5. Topology control
    pub_ctrl = s.declare_publisher("sim/network/control")

    time.sleep(2)

    logger.info("Making nodes known...")
    # Nodes must transmit to be known
    pub_eth_tx2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    pub_uart_tx2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    pub_sysc_tx2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    pub_rf_tx1.put(pack_rf_header(0, 0, 0, 0))  # Node 1 is at (10,0,0), Node 0 is at (0,0,0)
    pub_rf_tx2.put(pack_rf_header(0, 0, 0, 0))  # Node 2 is at (100,0,0)

    time.sleep(1)

    logger.info("Testing ETH...")
    pub_eth_tx1.put(vproto.ZenohFrameHeader(1000, 0, 4).pack() + b"ETH1")
    try:
        data = rx_eth.get(timeout=timeout_val)
        logger.info(f"data len: {len(data)}")
        hdr = vproto.ZenohFrameHeader.unpack(data)
        vtime = hdr.delivery_vtime_ns
        if vtime == 1001000:  # default 1ms delay
            results["eth"] = True
            logger.info("  ETH PASS")
        else:
            logger.info(f"  ETH FAIL: vtime={vtime}")
    except queue.Empty:
        logger.info("  ETH FAIL: no frame")

    logger.info("Testing UART...")
    pub_uart_tx1.put(vproto.ZenohFrameHeader(2000, 0, 4).pack() + b"UART")
    try:
        data = rx_uart.get(timeout=timeout_val)
        logger.info(f"data len: {len(data)}")
        hdr = vproto.ZenohFrameHeader.unpack(data)
        vtime = hdr.delivery_vtime_ns
        if vtime == 1002000:
            results["uart"] = True
            logger.info("  UART PASS")
        else:
            logger.info(f"  UART FAIL: vtime={vtime}")
    except queue.Empty:
        logger.info("  UART FAIL: no frame")

    logger.info("Testing SystemC...")
    # SystemC CAN uses ZenohFrameHeader FlatBuffer
    pub_sysc_tx1.put(vproto.ZenohFrameHeader(3000, 0, 4).pack() + b"SYSC")
    try:
        data = rx_sysc.get(timeout=timeout_val)
        logger.info(f"data len: {len(data)}")
        hdr = vproto.ZenohFrameHeader.unpack(data)
        vtime = hdr.delivery_vtime_ns
        if vtime == 1003000:  # default 1ms delay (1,000,000 ns) added to base 3000
            results["sysc"] = True
            logger.info("  SystemC PASS")
        else:
            logger.info(f"  SystemC FAIL: vtime={vtime}")
    except queue.Empty:
        logger.info("  SystemC FAIL: no frame")

    logger.info("Testing RF...")
    while not rx_rf.empty():
        rx_rf.get_nowait()
    # Node 0 to Node 1. Dist = 10m.
    # fspl = 20*log10(10) + 20*log10(2.4e9) + 20*log10(4*pi/c)
    # fspl = 20 + 187.6 - 147.5 = 60.1 dB
    # RSSI = 0 - 60.1 = -60.1 dBm
    pub_rf_tx0.put(pack_rf_header(4000, 4, 0, 0) + b"RF01")
    try:
        data = rx_rf.get(timeout=timeout_val)
        data = data
        vtime, size, rssi, lqi, offset = unpack_rf_header(data)  # noqa: RUF059
        logger.info(f"  RF received: vtime={vtime}, rssi={rssi}")
        if vtime >= 4000 + 1000000:  # 1ms + speed of light (33ns)
            results["rf"] = True
            logger.info("  RF PASS")
    except queue.Empty:
        logger.info("  RF FAIL: no frame")

    logger.info("Testing Overflow...")
    orig_vtime = 0xFFFFFFFFFFFFFFFF - 500000
    while not rx_eth.empty():
        rx_eth.get_nowait()
    pub_eth_tx1.put(vproto.ZenohFrameHeader(orig_vtime, 0, 4).pack() + b"OVER")
    try:
        data = rx_eth.get(timeout=timeout_val)
        logger.info(f"data len: {len(data)}")
        hdr = vproto.ZenohFrameHeader.unpack(data)
        vtime = hdr.delivery_vtime_ns
        if vtime >= orig_vtime:
            results["overflow"] = True
            logger.info("  Overflow PASS")
        else:
            logger.info(f"  Overflow FAIL: vtime={vtime} wrapped!")
    except queue.Empty:
        logger.info("  Overflow FAIL: no frame")

    logger.info("Testing RF Sensitivity...")
    # Node 0 (0,0,0) to Node 2 (100,0,0). Distance = 100m.
    # fspl = 20*log10(100) + 40.04 = 80.04 dB. RSSI = -80.04 dBm.
    # Default sensitivity is -90.0 dBm, so it should be received!
    rx_rf2 = queue.Queue()
    s.declare_subscriber("sim/rf/ieee802154/2/rx", lambda sample: rx_rf2.put(sample.payload.to_bytes()))
    time.sleep(0.5)
    pub_rf_tx0.put(pack_rf_header(8000, 4, 0, 0) + b"RF02")
    try:
        data = rx_rf2.get(timeout=timeout_val)
        vtime, size, rssi, _lqi, _offset = unpack_rf_header(data)  # noqa: RUF059
        logger.info(f"  RF Sensitivity PASS: frame received with rssi={rssi}")
        if rssi == -80:
            results["rf_sensitivity"] = True
    except queue.Empty:
        logger.info("  RF Sensitivity FAIL: frame dropped unexpectedly")

    logger.info("Testing RF HCI (no RF header)...")
    rx_hci = queue.Queue()
    s.declare_subscriber("sim/rf/hci/1/rx", lambda sample: rx_hci.put(sample.payload.to_bytes()))
    pub_hci_tx0 = s.declare_publisher("sim/rf/hci/0/tx")
    pub_hci_tx1 = s.declare_publisher("sim/rf/hci/1/tx")
    pub_hci_tx1.put(vproto.ZenohFrameHeader(0, 0, 0).pack())  # known
    time.sleep(0.5)
    pub_hci_tx0.put(vproto.ZenohFrameHeader(7000, 0, 4).pack() + b"HCI0")
    try:
        data = rx_hci.get(timeout=timeout_val)
        data = data
        logger.info(f"data len: {len(data)}")
        hdr = vproto.ZenohFrameHeader.unpack(data)
        vtime = hdr.delivery_vtime_ns
        if vtime >= 7000 + 1000000:
            logger.info("  RF HCI PASS")
            results["rf_hci"] = True
        else:
            logger.info(f"  RF HCI FAIL: vtime={vtime}")
    except queue.Empty:
        logger.info("  RF HCI FAIL: no frame")
    logger.info("Testing Mismatched Size (Malformed)...")
    while not rx_eth.empty():
        rx_eth.get_nowait()
    # Header says size 100, but we send only 4 bytes of data
    pub_eth_tx1.put(vproto.ZenohFrameHeader(9000, 0, 100).pack() + b"ABCD")
    try:
        rx_eth.get(timeout=malformed_timeout)
        logger.info("  Mismatched Size FAIL: malformed packet was forwarded")
    except queue.Empty:
        logger.info("  Mismatched Size PASS")
        results["malformed"] = True
    update = {"from": "1", "to": "2", "drop_probability": 1.0}
    pub_ctrl.put(json.dumps(update))
    time.sleep(0.5)
    while not rx_eth.empty():
        rx_eth.get_nowait()
    pub_eth_tx1.put(vproto.ZenohFrameHeader(5000, 0, 4).pack() + b"DROP")
    try:
        rx_eth.get(timeout=malformed_timeout)
        logger.info("  Topology FAIL: frame not dropped")
    except queue.Empty:
        logger.info("  Topology (Drop) PASS")
        # Now reset
        update = {"from": "1", "to": "2", "drop_probability": 0.0}
        pub_ctrl.put(json.dumps(update))
        time.sleep(0.5)
        pub_eth_tx1.put(vproto.ZenohFrameHeader(6000, 0, 4).pack() + b"KEPT")
        time.sleep(0.5)
        try:
            rx_eth.get(timeout=timeout_val)
            results["topology"] = True
            logger.info("  Topology (Reset) PASS")
        except queue.Empty:
            logger.info("  Topology FAIL: frame still dropped after reset")

    s.close()

    all_pass = all(results.values())
    if all_pass:
        logger.info("\nALL TESTS PASSED")
        s.close()
        sys.exit(0)
    else:
        logger.info(f"\nSOME TESTS FAILED: {results}")
        s.close()
        sys.exit(1)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
