import os
import struct
import sys
import time

import zenoh

# Protocol: 8 bytes vtime, 4 bytes size, 1 byte RSSI, 1 byte LQI
RF_HEADER_FORMAT = "<QIBB"
RF_HEADER_SIZE = 14

session = None
ping_responded = False
script_dir = os.path.dirname(os.path.abspath(__file__))

def on_sample(sample):
    global session, ping_responded
    payload = sample.payload.to_bytes()
    if len(payload) < RF_HEADER_SIZE:
        return

    header = struct.unpack(RF_HEADER_FORMAT, payload[:RF_HEADER_SIZE])
    vtime, size, rssi, lqi = header
    data = payload[RF_HEADER_SIZE:]

    # 802.15.4 FCF: bits 0-2 are frame type. Type 2 is ACK.
    if size >= 2:
        fcf = struct.unpack("<H", data[:2])[0]
        if (fcf & 0x07) == 0x02:
            return

    if ping_responded:
        return
    ping_responded = True

    print(f"[{vtime}] Received RF packet: size={size} RSSI={rssi} LQI={lqi}")

    # 1. Respond with WRONG address after 1ms virtual time
    resp1_vtime = vtime + 1000000
    resp1_data = struct.pack("<HBH HH H", 0x8841, 0x02, 0xABCD, 0x5678, 0x1234, 0) + b"MISMATCHED ACK"
    msg1 = struct.pack(RF_HEADER_FORMAT, resp1_vtime, len(resp1_data), 0xCE, 0xFF) + resp1_data
    print(f"[{resp1_vtime}] Sending MISMATCHED response...")
    session.put("sim/rf/802154/0/rx", msg1)

    # 2. Respond with CORRECT address after 2ms virtual time
    resp2_vtime = vtime + 2000000
    resp2_data = struct.pack("<HBH HH H", 0x8861, 0x03, 0xABCD, 0x1234, 0x5678, 0) + b"MATCHED ACK"
    msg2 = struct.pack(RF_HEADER_FORMAT, resp2_vtime, len(resp2_data), 0xCE, 0xFF) + resp2_data
    print(f"[{resp2_vtime}] Sending MATCHED response...")
    session.put("sim/rf/802154/0/rx", msg2)

def on_tx_sample(sample):
    payload = sample.payload.to_bytes()
    if len(payload) < RF_HEADER_SIZE:
        return

    header = struct.unpack(RF_HEADER_FORMAT, payload[:RF_HEADER_SIZE])
    vtime, size, rssi, lqi = header
    data = payload[RF_HEADER_SIZE:]

    if size == 3 and (data[0] & 0x07) == 0x02:
        print(f"[{vtime}] RECEIVED AUTO-ACK for seq {data[2]}")
        with open(os.path.join(script_dir, "ack_received.tmp"), "w") as f:
            f.write("OK")

def main():
    global session
    node_id = sys.argv[1] if len(sys.argv) > 1 else "0"
    router = sys.argv[2] if len(sys.argv) > 2 else "tcp/127.0.0.1:7448"

    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", f'["{router}"]')
    session = zenoh.open(conf)

    sub_topic = f"sim/rf/802154/{node_id}/tx"
    print(f"Listening on {sub_topic}...")
    session.declare_subscriber(sub_topic, on_sample)
    session.declare_subscriber(sub_topic, on_tx_sample)

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass

if __name__ == "__main__":
    main()
