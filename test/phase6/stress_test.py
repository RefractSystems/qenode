import os
import threading
import time

import vproto
import zenoh


def node_thread(node_id, num_messages, session):
    pub = session.declare_publisher(f"sim/eth/frame/{node_id}/tx")
    for i in range(num_messages):
        vtime = i * 1000
        payload = b"X" * 64
        pub.put(vproto.ZenohFrameHeader(vtime, 0, len(payload)).pack() + payload)
        # time.sleep(0.001)


def main():
    conf = zenoh.Config()

    router = os.environ.get("ZENOH_ROUTER")
    if router:
        conf.insert_json5("mode", '"client"')
        conf.insert_json5("connect/endpoints", f'["{router}"]')

    s = zenoh.open(conf)

    num_nodes = 20
    msgs_per_node = 50

    # First make them all known
    pubs = []
    for i in range(num_nodes):
        p = s.declare_publisher(f"sim/eth/frame/{i}/tx")
        p.put(vproto.ZenohFrameHeader(0, 0, 0).pack())
        pubs.append(p)

    time.sleep(2)

    threads = []
    start_time = time.time()
    for i in range(num_nodes):
        t = threading.Thread(target=node_thread, args=(i, msgs_per_node, s))
        t.start()
        threads.append(t)

    for t in threads:
        t.join()

    end_time = time.time()
    print(f"Sent {num_nodes * msgs_per_node} messages in {end_time - start_time:.2f} seconds")

    time.sleep(2)
    s.close()
    print("Stress test finished.")


if __name__ == "__main__":
    main()
