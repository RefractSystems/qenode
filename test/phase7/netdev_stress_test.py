import subprocess
import time
import struct
import sys
import zenoh
import threading
import os

WORKSPACE_DIR = "/workspace"

def pack_zenoh_frame(vtime_ns: int, data: bytes) -> bytes:
    header = struct.pack("<QI", vtime_ns, len(data))
    return header + data

def main():
    print("Starting Zenoh router...")
    router_proc = subprocess.Popen([sys.executable, os.path.join(WORKSPACE_DIR, "tests", "zenoh_router_persistent.py"), "tcp/127.0.0.1:7448"])
    time.sleep(2)

    print("Starting QEMU...")
    qemu_cmd = [
        os.path.join(WORKSPACE_DIR, "scripts", "run.sh"),
        "--dtb", os.path.join(WORKSPACE_DIR, "test-results", "netdev_determinism", "board.dtb"),
        "-kernel", os.path.join(WORKSPACE_DIR, "test-results", "netdev_determinism", "firmware.elf"),
        "-icount", "shift=0,align=off,sleep=off",
        "-netdev", "zenoh,id=net0,node=0,router=tcp/127.0.0.1:7448",
        "-nographic", "-monitor", "none"
    ]
    
    qemu_proc = subprocess.Popen(qemu_cmd, stderr=subprocess.PIPE, text=True)
    
    # Wait for QEMU to boot and subscribe to the topic
    time.sleep(3)

    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7448"]')
    conf.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(conf)

    rx_topic = "sim/eth/frame/0/rx"
    
    print("Injecting 1000 packets out of order...")
    base_time = 1_000_000_000 # 1 second in ns
    for i in range(1000):
        # Reverse order: first packet sent has the largest vtime
        vtime = base_time + (1000 - i) * 1000 
        data = f"PACKET_{i}".encode("utf-8")
        session.put(rx_topic, pack_zenoh_frame(vtime, data))

    print("Waiting for deliveries...")
    delivered_vtimes = []
    deadline = time.time() + 15.0
    while time.time() < deadline:
        line = qemu_proc.stderr.readline()
        if not line:
            break
        if "[virtmcu-netdev] RX deliver" in line:
            parts = line.split()
            vtime_str = [p for p in parts if p.startswith("vtime=")][0]
            vtime = int(vtime_str.split("=")[1])
            delivered_vtimes.append(vtime)
            if len(delivered_vtimes) == 1000:
                break

    qemu_proc.terminate()
    qemu_proc.wait()
    router_proc.terminate()
    router_proc.wait()

    if len(delivered_vtimes) != 1000:
        print(f"FAIL: Only delivered {len(delivered_vtimes)}/1000 packets.")
        sys.exit(1)

    if delivered_vtimes == sorted(delivered_vtimes):
        print("PASS: 1000 packets delivered in perfect virtual-time order despite being injected in reverse order!")
        sys.exit(0)
    else:
        print("FAIL: Packets delivered out of order!")
        sys.exit(1)

if __name__ == "__main__":
    main()
