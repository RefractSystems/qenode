import struct
import sys

import zenoh

# typedef struct __attribute__((packed)) {
#     uint64_t timestamp_ns;
#     uint8_t  type;
#     uint32_t id;
#     uint32_t value;
# } TraceEvent;
EVENT_FMT = "<Q B I I"
EVENT_SIZE = struct.calcsize(EVENT_FMT)


def decode_id(ev_type, ev_id):
    if ev_type == 1:  # TRACE_EVENT_IRQ: upper 16 bits = dev_slot, lower 16 = pin
        dev_slot = (ev_id >> 16) & 0xFFFF
        pin = ev_id & 0xFFFF
        slot_str = "?" if dev_slot == 0xFFFF else str(dev_slot)
        return f"dev={slot_str} pin={pin}"
    return f"id={ev_id}"


def on_sample(sample):
    payload = sample.payload.to_bytes()
    if len(payload) == EVENT_SIZE:
        ts, ev_type, ev_id, val = struct.unpack(EVENT_FMT, payload)
        if ev_type == 0:  # CPU_STATE
            type_str = "CPU_STATE"
            id_str = f"cpu={ev_id}"
        elif ev_type == 1:  # IRQ
            type_str = "IRQ"
            slot = ev_id >> 16
            pin = ev_id & 0xFFFF
            id_str = f"slot={slot:2} pin={pin:2}"
        elif ev_type == 2:  # PERIPHERAL
            type_str = "PERIPHERAL"
            id_str = f"id={ev_id}"
        else:
            type_str = "UNKNOWN"
            id_str = f"id={ev_id}"

        print(f"[{ts:15}] {type_str:10} {id_str} val={val:3}")
    else:
        print(f"Received malformed payload of size {len(payload)}: {payload.hex()}")


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
