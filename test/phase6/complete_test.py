import json
import struct
import sys
import time

import zenoh


def main():
    conf = zenoh.Config()
    s = zenoh.open(conf)

    results = {"eth": False, "uart": False, "sysc": False, "rf": False, "rf_hci": False, "rf_sensitivity": False, "overflow": False, "topology": False, "malformed": False}

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
    s.declare_subscriber("sim/rf/802154/1/rx", lambda sample: rx_rf.append(sample.payload.to_bytes()))
    pub_rf_tx0 = s.declare_publisher("sim/rf/802154/0/tx")
    pub_rf_tx1 = s.declare_publisher("sim/rf/802154/1/tx")

    # 5. Topology control
    pub_ctrl = s.declare_publisher("sim/network/control")

    time.sleep(2)

    print("Making nodes known...")
    # Nodes must transmit to be known
    pub_eth_tx2.put(struct.pack("<QI", 0, 0))
    pub_uart_tx2.put(struct.pack("<QI", 0, 0))
    pub_sysc_tx2.put(struct.pack("<QI", 0, 0))
    pub_rf_tx1.put(struct.pack("<QI", 0, 0)) # Node 1 is at (10,0,0), Node 0 is at (0,0,0)

    time.sleep(1)

    print("Testing ETH...")
    pub_eth_tx1.put(struct.pack("<QI", 1000, 4) + b"ETH1")
    time.sleep(0.5)
    if len(rx_eth) > 0:
        vtime, size = struct.unpack("<QI", rx_eth[0][:12])
        if vtime == 1001000: # default 1ms delay
            results["eth"] = True
            print("  ETH PASS")
        else:
            print(f"  ETH FAIL: vtime={vtime}")
    else:
        print("  ETH FAIL: no frame")

    print("Testing UART...")
    pub_uart_tx1.put(struct.pack("<QI", 2000, 4) + b"UART")
    time.sleep(0.5)
    if len(rx_uart) > 0:
        vtime, size = struct.unpack("<QI", rx_uart[0][:12])
        if vtime == 1002000:
            results["uart"] = True
            print("  UART PASS")
        else:
            print(f"  UART FAIL: vtime={vtime}")
    else:
        print("  UART FAIL: no frame")

    print("Testing SystemC...")
    pub_sysc_tx1.put(struct.pack("<QI", 3000, 4) + b"SYSC")
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
    # Node 0 to Node 1. Dist = 10m.
    # fspl = 20*log10(10) + 20*log10(2.4e9) + 20*log10(4*pi/c)
    # fspl = 20 + 187.6 - 147.5 = 60.1 dB
    # RSSI = 0 - 60.1 = -60.1 dBm
    # INPUT header to coordinator must be 14 bytes if has_rf_header=True
    # (vtime: 8, size: 4, rssi: 1, lqi: 1)
    pub_rf_tx0.put(struct.pack("<QIbb", 4000, 4, 0, 0) + b"RF01")
    time.sleep(0.5)
    if len(rx_rf) > 0:
        data = rx_rf[0]
        vtime, size, rssi, lqi = struct.unpack("<QIbb", data[:14])
        print(f"  RF received: vtime={vtime}, rssi={rssi}")
        if vtime >= 4000 + 1000000: # 1ms + speed of light (33ns)
            results["rf"] = True
            print("  RF PASS")
    else:
        print("  RF FAIL: no frame")

    print("Testing Overflow...")
    orig_vtime = 0xFFFFFFFFFFFFFFFF - 500000
    rx_eth.clear()
    pub_eth_tx1.put(struct.pack("<QI", orig_vtime, 4) + b"OVER")
    time.sleep(0.5)
    if len(rx_eth) > 0:
        vtime, size = struct.unpack("<QI", rx_eth[0][:12])
        if vtime >= orig_vtime:
            results["overflow"] = True
            print("  Overflow PASS")
        else:
            print(f"  Overflow FAIL: vtime={vtime} wrapped!")
    else:
        print("  Overflow FAIL: no frame")

    print("Testing RF Sensitivity (Drop)...")
    # Node 0 (0,0,0) to Node 2 (100,0,0). RSSI = -80.1 dBm.
    # Sensitivity is -70.0 dBm (if started with that flag).
    rx_rf2 = []
    s.declare_subscriber("sim/rf/802154/2/rx", lambda sample: rx_rf2.append(sample.payload.to_bytes()))
    pub_rf_tx0.put(struct.pack("<QIbb", 8000, 4, 0, 0) + b"RF02")
    time.sleep(0.5)
    if len(rx_rf2) == 0:
        print("  RF Sensitivity Drop PASS (if sensitivity > -80.1)")
        results["rf_sensitivity"] = True
    else:
        vtime, size, rssi, lqi = struct.unpack("<QIbb", rx_rf2[0][:14])
        print(f"  RF Sensitivity Drop FAIL: frame received with rssi={rssi}")

    print("Testing RF HCI (no RF header)...")
    rx_hci = []
    s.declare_subscriber("sim/rf/hci/1/rx", lambda sample: rx_hci.append(sample.payload.to_bytes()))
    pub_hci_tx0 = s.declare_publisher("sim/rf/hci/0/tx")
    pub_hci_tx1 = s.declare_publisher("sim/rf/hci/1/tx")
    pub_hci_tx1.put(struct.pack("<QI", 0, 0)) # known
    time.sleep(0.5)
    pub_hci_tx0.put(struct.pack("<QI", 7000, 4) + b"HCI0")
    time.sleep(0.5)
    if len(rx_hci) > 0:
        data = rx_hci[0]
        vtime, size = struct.unpack("<QI", data[:12])
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
    pub_eth_tx1.put(struct.pack("<QI", 9000, 100) + b"ABCD")
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
    pub_eth_tx1.put(struct.pack("<QI", 5000, 4) + b"DROP")
    time.sleep(0.5)
    if len(rx_eth) == 0:
        print("  Topology (Drop) PASS")
        # Now reset
        update = {"from": "1", "to": "2", "drop_probability": 0.0}
        pub_ctrl.put(json.dumps(update))
        time.sleep(0.5)
        pub_eth_tx1.put(struct.pack("<QI", 6000, 4) + b"KEPT")
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
