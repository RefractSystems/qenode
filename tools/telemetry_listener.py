import zenoh
import struct
import sys

# typedef struct __attribute__((packed)) {
#     uint64_t timestamp_ns;
#     uint8_t  type;
#     uint32_t id;
#     uint32_t value;
# } TraceEvent;
EVENT_FMT = "<Q B I I"
EVENT_SIZE = struct.calcsize(EVENT_FMT)

def on_sample(sample):
    payload = sample.payload.to_bytes()
    if len(payload) == EVENT_SIZE:
        ts, ev_type, ev_id, val = struct.unpack(EVENT_FMT, payload)
        type_str = ["CPU_STATE", "IRQ", "PERIPHERAL"][ev_type] if ev_type < 3 else "UNKNOWN"
        print(f"[{ts:15}] {type_str:10} id={ev_id:3} val={val:3}")
    else:
        print(f"Received malformed payload of size {len(payload)}")

if __name__ == "__main__":
    node_id = sys.argv[1] if len(sys.argv) > 1 else "0"
    topic = f"sim/telemetry/trace/{node_id}"
    print(f"Listening on {topic}...")
    
    session = zenoh.open(zenoh.Config())
    sub = session.declare_subscriber(topic, on_sample)
    
    try:
        import time
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
