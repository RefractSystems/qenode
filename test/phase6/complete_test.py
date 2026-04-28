import json
import os
import struct
import sys
import time
from pathlib import Path

import vproto

sys.path.append(Path(Path(__file__).resolve().parent))
import flatbuffers
import zenoh
from virtmcu.rf import RfHeader


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
    sz = struct.unpack("<I", data[:4])[0]
    hdr = RfHeader.RfHeader.GetRootAs(data[4 : 4 + sz], 0)
    return hdr.DeliveryVtimeNs(), hdr.Size(), hdr.Rssi(), hdr.Lqi(), 4 + sz


def main():
    conf = zenoh.Config()

    router = os.environ.get("ZENOH_ROUTER")
    if router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{router}"]')

    s = zenoh.open(conf)

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
    rx_eth = []
    s.declare_subscriber("sim/eth/frame/2/rx", lambda sample: rx_eth.append(sample.payload.to_bytes()))
    pub_eth_tx1 = s.declare_publisher("sim/eth/frame/1/tx")
    pub_eth_tx2 = s.declare_publisher("sim/eth/frame/2/tx")

    # 2. UART test
    rx_uart = []
    s.declare_subscriber("virtmcu/uart/2/rx", lambda sample: rx_uart.append(sample.payload.to_bytes()))
    pub_uart_tx1 = s.declare_publisher("virtmcu/uart/1/tx")
    pub_uart_tx2 = s.declare_publisher("virtmcu/uart/2/tx")

    # 3. SystemC test
    rx_sysc = []
    s.declare_subscriber("sim/systemc/frame/2/rx", lambda sample: rx_sysc.append(sample.payload.to_bytes()))
    pub_sysc_tx1 = s.declare_publisher("sim/systemc/frame/1/tx")
    pub_sysc_tx2 = s.declare_publisher("sim/systemc/frame/2/tx")

    # 4. RF test (802.15.4)
    rx_rf = []
    s.declare_subscriber("sim/rf/ieee802154/1/rx", lambda sample: rx_rf.append(sample.payload.to_bytes()))
    pub_rf_tx0 = s.declare_publisher("sim/rf/ieee802154/0/tx")
    pub_rf_tx1 = s.declare_publisher("sim/rf/ieee802154/1/tx")
    pub_rf_tx2 = s.declare_publisher("sim/rf/ieee802154/2/tx")

    # 5. Topology control
    pub_ctrl = s.declare_publisher("sim/network/control")

    time.sleep(2)

    print("Making nodes known...")
    # Nodes must transmit to be known
    pub_eth_tx2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    pub_uart_tx2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    pub_sysc_tx2.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
    pub_rf_tx1.put(pack_rf_header(0, 0, 0, 0))  # Node 1 is at (10,0,0), Node 0 is at (0,0,0)
    pub_rf_tx2.put(pack_rf_header(0, 0, 0, 0))  # Node 2 is at (100,0,0)

    time.sleep(1)

    print("Testing ETH...")
    pub_eth_tx1.put(vproto.ZenohFrameHeader(1000, 0, 4).pack() + b"ETH1")
    time.sleep(0.5)
    if len(rx_eth) > 0:
        vtime, _seq, size = struct.unpack("<QQI", rx_eth[0][:20])
        if vtime == 1001000:  # default 1ms delay
            results["eth"] = True
            print("  ETH PASS")
        else:
            print(f"  ETH FAIL: vtime={vtime}")
    else:
        print("  ETH FAIL: no frame")

    print("Testing UART...")
    pub_uart_tx1.put(vproto.ZenohFrameHeader(2000, 0, 4).pack() + b"UART")
    time.sleep(0.5)
    if len(rx_uart) > 0:
        vtime, _seq, size = struct.unpack("<QQI", rx_uart[0][:20])
        if vtime == 1002000:
            results["uart"] = True
            print("  UART PASS")
        else:
            print(f"  UART FAIL: vtime={vtime}")
    else:
        print("  UART FAIL: no frame")

    print("Testing SystemC...")
    pub_sysc_tx1.put(vproto.ZenohFrameHeader(3000, 0, 4).pack() + b"SYSC")
    time.sleep(0.5)
    if len(rx_sysc) > 0:
        vtime, size = struct.unpack("<QI", rx_sysc[0][:12])
        if vtime == 1003000:
            results["sysc"] = True
            print("  SystemC PASS")
        else:
            print(f"  SystemC FAIL: vtime={vtime}")
    else:
        print("  SystemC FAIL: no frame")

    print("Testing RF...")
    rx_rf.clear()
    # Node 0 to Node 1. Dist = 10m.
    # fspl = 20*log10(10) + 20*log10(2.4e9) + 20*log10(4*pi/c)
    # fspl = 20 + 187.6 - 147.5 = 60.1 dB
    # RSSI = 0 - 60.1 = -60.1 dBm
    pub_rf_tx0.put(pack_rf_header(4000, 4, 0, 0) + b"RF01")
    time.sleep(0.5)
    if len(rx_rf) > 0:
        data = rx_rf[0]
        vtime, size, rssi, lqi, offset = unpack_rf_header(data)  # noqa: RUF059
        print(f"  RF received: vtime={vtime}, rssi={rssi}")
        if vtime >= 4000 + 1000000:  # 1ms + speed of light (33ns)
            results["rf"] = True
            print("  RF PASS")
    else:
        print("  RF FAIL: no frame")

    print("Testing Overflow...")
    orig_vtime = 0xFFFFFFFFFFFFFFFF - 500000
    rx_eth.clear()
    pub_eth_tx1.put(vproto.ZenohFrameHeader(orig_vtime, 0, 4).pack() + b"OVER")
    time.sleep(0.5)
    if len(rx_eth) > 0:
        vtime, _seq, size = struct.unpack("<QQI", rx_eth[0][:20])
        if vtime >= orig_vtime:
            results["overflow"] = True
            print("  Overflow PASS")
        else:
            print(f"  Overflow FAIL: vtime={vtime} wrapped!")
    else:
        print("  Overflow FAIL: no frame")

    print("Testing RF Sensitivity...")
    # Node 0 (0,0,0) to Node 2 (100,0,0). Distance = 100m.
    # fspl = 20*log10(100) + 40.04 = 80.04 dB. RSSI = -80.04 dBm.
    # Default sensitivity is -90.0 dBm, so it should be received!
    rx_rf2 = []
    s.declare_subscriber("sim/rf/ieee802154/2/rx", lambda sample: rx_rf2.append(sample.payload.to_bytes()))
    time.sleep(0.5)
    pub_rf_tx0.put(pack_rf_header(8000, 4, 0, 0) + b"RF02")
    time.sleep(0.5)
    if len(rx_rf2) > 0:
        vtime, size, rssi, _lqi, _offset = unpack_rf_header(rx_rf2[0])  # noqa: RUF059
        print(f"  RF Sensitivity PASS: frame received with rssi={rssi}")
        if rssi == -80:
            results["rf_sensitivity"] = True
    else:
        print("  RF Sensitivity FAIL: frame dropped unexpectedly")

    print("Testing RF HCI (no RF header)...")
    rx_hci = []
    s.declare_subscriber("sim/rf/hci/1/rx", lambda sample: rx_hci.append(sample.payload.to_bytes()))
    pub_hci_tx0 = s.declare_publisher("sim/rf/hci/0/tx")
    pub_hci_tx1 = s.declare_publisher("sim/rf/hci/1/tx")
    pub_hci_tx1.put(vproto.ZenohFrameHeader(0, 0, 0).pack())  # known
    time.sleep(0.5)
    pub_hci_tx0.put(vproto.ZenohFrameHeader(7000, 0, 4).pack() + b"HCI0")
    time.sleep(0.5)
    if len(rx_hci) > 0:
        data = rx_hci[0]
        vtime, _seq, _size = struct.unpack("<QQI", data[:vproto.SIZE_ZENOH_FRAME_HEADER])
        if vtime >= 7000 + 1000000:
            print("  RF HCI PASS")
            results["rf_hci"] = True
        else:
            print(f"  RF HCI FAIL: vtime={vtime}")
    else:
        print("  RF HCI FAIL: no frame")
    print("Testing Mismatched Size (Malformed)...")
    rx_eth.clear()
    # Header says size 100, but we send only 4 bytes of data
    pub_eth_tx1.put(vproto.ZenohFrameHeader(9000, 0, 100).pack() + b"ABCD")
    time.sleep(0.5)
    if len(rx_eth) == 0:
        print("  Mismatched Size PASS")
        results["malformed"] = True
    else:
        print("  Mismatched Size FAIL: malformed packet was forwarded")
    update = {"from": "1", "to": "2", "drop_probability": 1.0}
    pub_ctrl.put(json.dumps(update))
    time.sleep(0.5)
    rx_eth.clear()
    pub_eth_tx1.put(vproto.ZenohFrameHeader(5000, 0, 4).pack() + b"DROP")
    time.sleep(0.5)
    if len(rx_eth) == 0:
        print("  Topology (Drop) PASS")
        # Now reset
        update = {"from": "1", "to": "2", "drop_probability": 0.0}
        pub_ctrl.put(json.dumps(update))
        time.sleep(0.5)
        pub_eth_tx1.put(vproto.ZenohFrameHeader(6000, 0, 4).pack() + b"KEPT")
        time.sleep(0.5)
        if len(rx_eth) > 0:
            results["topology"] = True
            print("  Topology (Reset) PASS")
        else:
            print("  Topology FAIL: frame still dropped after reset")
    else:
        print("  Topology FAIL: frame not dropped")

    s.close()

    all_pass = all(results.values())
    if all_pass:
        print("\nALL PHASE 6 TESTS PASSED")
        sys.exit(0)
    else:
        print(f"\nSOME TESTS FAILED: {results}")
        sys.exit(1)


if __name__ == "__main__":
    main()
