"""
test/phase7/netdev_determinism_test.py

Determinism test for zenoh-netdev's priority-queue RX path (task 7.8).

Scenario
--------
Two Ethernet frames are published to the QEMU node's Zenoh RX topic in
*reverse* virtual-time order:
  - Packet A  vtime = now + 2 ms  (published first)
  - Packet B  vtime = now + 1 ms  (published second)

The clock is then advanced past both vtimes (by 3 ms).  The Rust
rx_timer_cb must drain the priority queue in vtime order, so Packet B
(now + 1 ms) must be delivered before Packet A (now + 2 ms).

Verification uses the log line emitted by rx_timer_cb:
    [virtmcu-netdev] RX deliver node=0 now=<ns> vtime=<ns> size=<bytes>

No guest NIC is required — the netdev backend operates independently and
the delivery ordering is observable purely from QEMU's stderr.
"""

import argparse
import os
import re
import struct
import subprocess
import sys
import time

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_DIR = os.path.dirname(os.path.dirname(SCRIPT_DIR))
TOOLS_DIR = os.path.join(WORKSPACE_DIR, "tools")
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

import zenoh  # noqa: E402
from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

# Frame sizes used to identify packets in the delivery log.
SIZE_A = 34   # Packet A: 14-byte Ethernet header + 20 payload bytes
SIZE_B = 24   # Packet B: 14-byte Ethernet header + 10 payload bytes

def pack_clock_req(delta_ns: int) -> bytes:
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0)
    return req.pack()


def pack_net_frame(vtime_ns: int, data: bytes) -> bytes:
    """Wire format expected by zenoh-netdev: 8-byte LE vtime + 4-byte LE size + payload."""
    return struct.pack("<QI", vtime_ns, len(data)) + data


def build_firmware(tmpdir: str) -> str:
    """Compile a minimal bare-metal ARM binary (infinite nop loop)."""
    linker = os.path.join(tmpdir, "linker.ld")
    asm    = os.path.join(tmpdir, "firmware.S")
    elf    = os.path.join(tmpdir, "firmware.elf")

    with open(linker, "w") as f:
        f.write("SECTIONS { . = 0x40000000; .text : { *(.text) } }\n")
    with open(asm, "w") as f:
        f.write(".global _start\n_start:\n" + "  nop\n"*100 + "  b _start\n")

    subprocess.run(
        ["arm-none-eabi-gcc", "-mcpu=cortex-a15", "-nostdlib",
         "-T", linker, asm, "-o", elf],
        check=True,
    )
    return elf


def build_dtb(tmpdir: str) -> str:
    """Generate a minimal DTB (Cortex-A15 + RAM only, no NIC in FDT)."""
    yaml_content = """\
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: ram
    type: Memory.MappedMemory
    address: 0x40000000
    properties:
      size: 0x10000000
"""
    yaml_path = os.path.join(tmpdir, "board.yaml")
    dtb_path  = os.path.join(tmpdir, "board.dtb")

    with open(yaml_path, "w") as f:
        f.write(yaml_content)

    subprocess.run(
        [sys.executable, "-m", "tools.yaml2qemu", yaml_path, "--out-dtb", dtb_path],
        cwd=WORKSPACE_DIR,
        check=True,
    )
    return dtb_path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--router", default="tcp/127.0.0.1:7448")
    args = parser.parse_args()

    tmpdir = os.path.join(WORKSPACE_DIR, "test-results", "netdev_determinism")
    os.makedirs(tmpdir, exist_ok=True)

    dtb_path = build_dtb(tmpdir)
    elf_path = build_firmware(tmpdir)

    # Kill any stale router from a previous run.
    subprocess.run(["pkill", "-f", "zenoh_router_persistent.py"], check=False)
    time.sleep(0.3)

    router_proc = subprocess.Popen(
        [sys.executable,
         os.path.join(WORKSPACE_DIR, "tests", "zenoh_router_persistent.py"),
         args.router],
    )
    time.sleep(1)

    # QEMU stderr is captured so we can parse delivery log lines.
    stderr_path = os.path.join(tmpdir, "qemu_stderr.log")
    stderr_file = open(stderr_path, "w")

    qemu_cmd = [
        os.path.join(WORKSPACE_DIR, "scripts", "run.sh"),
        "--dtb", dtb_path,
        "-kernel", elf_path,
        "-icount", "shift=0,align=off,sleep=off",
        "-device", f"zenoh-clock,mode=slaved-icount,node=0,router={args.router}",
        "-netdev", f"zenoh,id=net0,node=0,router={args.router}",
        "-nographic", "-monitor", "none"
    ]

    print(f"Running: {' '.join(qemu_cmd)}")
    qemu_proc = subprocess.Popen(qemu_cmd, stderr=subprocess.STDOUT, stdout=stderr_file)

    try:
        cfg = zenoh.Config()
        cfg.insert_json5("connect/endpoints", f'["{args.router}"]')
        cfg.insert_json5("scouting/multicast/enabled", "false")
        session = zenoh.open(cfg)

        # Wait for the clock queryable (signals QEMU is ready).
        clock_topic = "sim/clock/advance/0"
        deadline = time.time() + 60
        ready = False
        now_ns = 0
        while time.time() < deadline:
            replies = list(session.get(
                clock_topic, payload=pack_clock_req(1_000_000), timeout=10.0))
            print("REPLIES:", replies)
            if replies:
                # replies is a list of zenoh.Reply
                for reply in replies:
                    if hasattr(reply, "ok") and reply.ok is not None:
                        payload = reply.ok.payload
                        resp = ClockReadyResp.unpack(payload.to_bytes())
                        now_ns = resp.current_vtime_ns
                        ready = True
                        break
                if ready:
                    break
            time.sleep(0.5)

        if not ready:
            print("FAIL: Could not get valid initial clock state.", file=sys.stderr)
            # Dump stderr for debugging
            stderr_file.flush()
            with open(stderr_path) as f:
                print("\n--- QEMU stderr ---")
                print(f.read())
            sys.exit(1)

        print(f"QEMU ready. now={now_ns} ns. Injecting out-of-order frames...")

        # Schedule frames in the next quantum (current now_ns is at least 1ms)
        VTIME_A_NS = now_ns + 200_000_000_000 # now + 200s
        VTIME_B_NS = now_ns + 100_000_000_000 # now + 100s
        CLOCK_ADV_NS = 300_000_000_000        # advance by 300s

        rx_topic = "sim/eth/frame/0/rx"
        eth_hdr  = bytes([0x66,0x77,0x88,0x99,0xAA,0xBB,
                          0x00,0x11,0x22,0x33,0x44,0x55,
                          0x08,0x00])

        packet_a = eth_hdr + b"A" * 20   # SIZE_A = 34
        packet_b = eth_hdr + b"B" * 10   # SIZE_B = 24

        # Deliberately publish A (later vtime) first to test re-ordering.
        session.put(rx_topic, pack_net_frame(VTIME_A_NS, packet_a))
        print(f"  Sent Packet A  vtime={VTIME_A_NS} ns  size={SIZE_A}  (published first)")
        time.sleep(0.5)
        session.put(rx_topic, pack_net_frame(VTIME_B_NS, packet_b))
        print(f"  Sent Packet B  vtime={VTIME_B_NS} ns  size={SIZE_B}  (published second)")

        # Advance clock past both vtimes — this drains the priority queue.
        print(f"Advancing clock by {CLOCK_ADV_NS} ns...")
        list(session.get(clock_topic,
                         payload=pack_clock_req(CLOCK_ADV_NS),
                         timeout=10.0))

        # Give rx_timer_cb a moment to flush its log to stderr.
        time.sleep(0.5)
        session.close()

    finally:
        qemu_proc.terminate()
        qemu_proc.wait()
        stderr_file.close()
        router_proc.terminate()
        router_proc.wait()

    # Parse delivery log lines from QEMU stderr.
    # Format: [virtmcu-netdev] RX deliver node=0 vtime=<ns> size=<bytes>
    log_re = re.compile(
        r"\[virtmcu-netdev\] RX deliver node=\d+ vtime=(\d+) size=(\d+)")

    deliveries = []
    with open(stderr_path) as f:
        for line in f:
            m = log_re.search(line)
            if m:
                deliveries.append((int(m.group(1)), int(m.group(2))))

    print("\nDelivery log (vtime_ns, size):")
    for vt, sz in deliveries:
        print(f"  vtime={vt}  size={sz}")

    if len(deliveries) < 2:
        print(f"FAIL: expected ≥ 2 delivery events, got {len(deliveries)}")
        # Dump stderr for debugging
        with open(stderr_path) as f:
            print("\n--- QEMU stderr ---")
            print(f.read())
        sys.exit(1)

    # Find the delivery index of each packet by size.
    idx_a = next((i for i, (_, sz) in enumerate(deliveries) if sz == SIZE_A), -1)
    idx_b = next((i for i, (_, sz) in enumerate(deliveries) if sz == SIZE_B), -1)

    if idx_a == -1 or idx_b == -1:
        print(f"FAIL: could not identify both packets. idx_a={idx_a} idx_b={idx_b}")
        sys.exit(1)

    if idx_b >= idx_a:
        print(f"FAIL: Packet B (vtime={VTIME_B_NS}) delivered at index {idx_b} "
              f"but Packet A (vtime={VTIME_A_NS}) delivered at index {idx_a}. "
              f"Expected B before A.")
        sys.exit(1)

    print(f"PASS: Packet B delivered before Packet A [idx_b={idx_b} < idx_a={idx_a}]")


if __name__ == "__main__":
    main()
