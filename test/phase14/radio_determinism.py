import zenoh
import struct
import time
import sys

# Protocol: 8 bytes vtime, 4 bytes size, 1 byte RSSI, 1 byte LQI
RF_HEADER_FORMAT = "<QIBB"
RF_HEADER_SIZE = 14

def on_sample(sample):
    payload = sample.payload.to_bytes()
    if len(payload) < RF_HEADER_SIZE:
        print(f"Payload too small: {len(payload)}")
        return
    
    vtime, size, rssi, lqi = struct.unpack(RF_HEADER_FORMAT, payload[:RF_HEADER_SIZE])
    data = payload[RF_HEADER_SIZE:RF_HEADER_SIZE+size]
    
    print(f"[{vtime}] Received RF packet: size={size} RSSI={rssi} LQI={lqi}")
    print(f"Data: {data}")

    # 1. Respond with WRONG address after 1ms virtual time
    resp1_vtime = vtime + 1000000
    resp1_data = struct.pack("<HBH HH H", 
        0x8841, # FCF
        0x02,   # Seq
        0xABCD, # Dest PAN
        0x5678, # Dest Addr (MISMATCH! Firmware expects 0x1234)
        0x1234, # Src Addr
        0) + b"MISMATCHED ACK"
    
    header1 = struct.pack(RF_HEADER_FORMAT, resp1_vtime, len(resp1_data), -50 & 0xFF, 255)
    print(f"[{resp1_vtime}] Sending MISMATCHED response to sim/rf/802154/0/rx...")
    session.put("sim/rf/802154/0/rx", header1 + resp1_data)

    # 2. Respond with MATCHING address after 2ms virtual time
    resp2_vtime = vtime + 2000000
    resp2_data = struct.pack("<HBH HH H", 
        0x8841, # FCF
        0x03,   # Seq
        0xABCD, # Dest PAN
        0x1234, # Dest Addr (MATCH!)
        0x1234, # Src Addr
        0) + b"MATCHED ACK"
    
    header2 = struct.pack(RF_HEADER_FORMAT, resp2_vtime, len(resp2_data), -50 & 0xFF, 255)
    print(f"[{resp2_vtime}] Sending MATCHED response to sim/rf/802154/0/rx...")
    session.put("sim/rf/802154/0/rx", header2 + resp2_data)

if __name__ == "__main__":
    node_id = sys.argv[1] if len(sys.argv) > 1 else "0"
    router = sys.argv[2] if len(sys.argv) > 2 else None
    
    config = zenoh.Config()
    if router:
        config.insert_json5("connect/endpoints", f'["{router}"]')
        config.insert_json5("scouting/multicast/enabled", "false")
    
    session = zenoh.open(config)
    
    sub_topic = "sim/rf/802154/0/tx"
    print(f"Listening on {sub_topic}...")
    sub = session.declare_subscriber(sub_topic, on_sample)
    
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
