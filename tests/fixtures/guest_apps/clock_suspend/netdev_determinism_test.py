"""
tests/fixtures/guest_apps/clock_suspend/netdev_determinism_test.py

Determinism test for netdev's priority-queue RX path (task 7.8).

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
import logging
import os
import re
import subprocess
import sys
import time
from pathlib import Path

import vproto


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent

SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE_DIR = _find_workspace_root(SCRIPT_DIR)
TOOLS_DIR = WORKSPACE_DIR / "tools"

if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import zenoh  # noqa: E402
from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

logger = logging.getLogger(__name__)

# Frame sizes used to identify packets in the delivery log.
SIZE_A = 34  # Packet A: 14-byte Ethernet header + 20 payload bytes
SIZE_B = 24  # Packet B: 14-byte Ethernet header + 10 payload bytes


Q_NUM = 0


def pack_clock_req(delta_ns: int) -> bytes:
    global Q_NUM
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=Q_NUM)
    Q_NUM += 1
    return req.pack()


def pack_net_frame(vtime_ns: int, data: bytes) -> bytes:
    """Wire format expected by netdev: 8-byte LE vtime + 4-byte LE size + payload."""
    return vproto.ZenohFrameHeader(vtime_ns, 0, len(data)).pack() + data


def build_firmware(tmpdir: str) -> str:
    """Compile a minimal bare-metal ARM binary (infinite nop loop)."""
    linker = Path(tmpdir) / "linker.ld"
    asm = Path(tmpdir) / "firmware.S"
    elf = Path(tmpdir) / "firmware.elf"

    with Path(linker).open("w") as f:
        f.write("SECTIONS { . = 0x40000000; .text : { *(.text) } }\n")
    with Path(asm).open("w") as f:
        f.write(".global _start\n_start:\n" + "  nop\n" * 100 + "  b _start\n")

    subprocess.run(
        ["arm-none-eabi-gcc", "-mcpu=cortex-a15", "-nostdlib", "-T", linker, asm, "-o", elf],
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
    yaml_path = Path(tmpdir) / "board.yaml"
    dtb_path = Path(tmpdir) / "board.dtb"

    with Path(yaml_path).open("w") as f:
        f.write(yaml_content)

    subprocess.run(
        [sys.executable, "-m", "tools.yaml2qemu", yaml_path, "--out-dtb", dtb_path],
        cwd=WORKSPACE_DIR,
        check=True,
    )
    return dtb_path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--router")
    args = parser.parse_args()

    if not args.router:
        port_script = Path(WORKSPACE_DIR) / "scripts" / "get-free-port.py"
        args.router = subprocess.check_output([sys.executable, str(port_script), "--endpoint", "--proto", "tcp/"]).decode().strip()

    tmpdir = Path(WORKSPACE_DIR) / "test-results" / "netdev_determinism"
    tmpdir.mkdir(parents=True, exist_ok=True)

    dtb_path = build_dtb(tmpdir)
    elf_path = build_firmware(tmpdir)

    router_proc = subprocess.Popen(
        [sys.executable, str(Path(WORKSPACE_DIR) / "tests" / "zenoh_router_persistent.py"), args.router],
    )
    time.sleep(1)

    # QEMU stderr is captured so we can parse delivery log lines.
    stderr_path = Path(tmpdir) / "qemu_stderr.log"
    with Path(stderr_path).open("w") as stderr_file:
        run_sh_path = os.environ.get("RUN_SH") or str(WORKSPACE_DIR / "scripts" / "run.sh")
        qemu_cmd = [
            run_sh_path,
            "--dtb",
            dtb_path,
            "-kernel",
            elf_path,
            "-icount",
            "shift=0,align=off,sleep=off",
            "-device",
            f"clock,mode=slaved-icount,node=0,router={args.router}",
            "-netdev",
            f"zenoh,id=net0,node=0,router={args.router}",
            "-nographic",
            "-monitor",
            "none",
        ]

        logger.info(f"Running: {' '.join(qemu_cmd)}")
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
                replies = list(session.get(clock_topic, payload=pack_clock_req(1_000_000), timeout=10.0))
                logger.info(f"REPLIES: {replies}")
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
                logger.error("FAIL: Could not get valid initial clock state.")
                # Dump stderr for debugging
                stderr_file.flush()
                with Path(stderr_path).open() as f:
                    logger.info("\n--- QEMU stderr ---")
                    logger.info(f.read())
                sys.exit(1)

            logger.info(f"QEMU ready. now={now_ns} ns. Injecting out-of-order frames...")

            # Schedule frames in the next quantum (current now_ns is at least 1ms)
            VTIME_A_NS = now_ns + 200_000_000_000  # now + 200s  # noqa: N806
            VTIME_B_NS = now_ns + 100_000_000_000  # now + 100s  # noqa: N806
            CLOCK_ADV_NS = 300_000_000_000  # advance by 300s  # noqa: N806

            rx_topic = "sim/eth/frame/0/rx"
            eth_hdr = bytes([0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x08, 0x00])

            packet_a = eth_hdr + b"A" * 20  # SIZE_A = 34
            packet_b = eth_hdr + b"B" * 10  # SIZE_B = 24

            # Deliberately publish A (later vtime) first to test re-ordering.
            session.put(rx_topic, pack_net_frame(VTIME_A_NS, packet_a))
            logger.info(f"  Sent Packet A  vtime={VTIME_A_NS} ns  size={SIZE_A}  (published first)")
            time.sleep(0.5)
            session.put(rx_topic, pack_net_frame(VTIME_B_NS, packet_b))
            logger.info(f"  Sent Packet B  vtime={VTIME_B_NS} ns  size={SIZE_B}  (published second)")

            # Advance clock past both vtimes — this drains the priority queue.
            logger.info(f"Advancing clock by {CLOCK_ADV_NS} ns...")
            list(session.get(clock_topic, payload=pack_clock_req(CLOCK_ADV_NS), timeout=10.0))

            # Give rx_timer_cb a moment to flush its log to stderr.
            time.sleep(0.5)
            session.close()

        finally:
            qemu_proc.terminate()
            qemu_proc.wait()
            router_proc.terminate()
            router_proc.wait()

    # Parse delivery log lines from QEMU stderr.
    # Format: [virtmcu-netdev] RX deliver node=0 vtime=<ns> size=<bytes>
    log_re = re.compile(r"\[virtmcu-netdev\] RX deliver node=\d+ vtime=(\d+) size=(\d+)")

    deliveries = []
    with Path(stderr_path).open() as f:
        for line in f:
            m = log_re.search(line)
            if m:
                deliveries.append((int(m.group(1)), int(m.group(2))))

    logger.info("\nDelivery log (vtime_ns, size):")
    for vt, sz in deliveries:
        logger.info(f"  vtime={vt}  size={sz}")

    if len(deliveries) < 2:
        logger.error(f"FAIL: expected ≥ 2 delivery events, got {len(deliveries)}")
        # Dump stderr for debugging
        with Path(stderr_path).open() as f:
            logger.info("\n--- QEMU stderr ---")
            logger.info(f.read())
        sys.exit(1)

    # Find the delivery index of each packet by size.
    idx_a = next((i for i, (_, sz) in enumerate(deliveries) if sz == SIZE_A), -1)
    idx_b = next((i for i, (_, sz) in enumerate(deliveries) if sz == SIZE_B), -1)

    if idx_a == -1 or idx_b == -1:
        logger.error(f"FAIL: could not identify both packets. idx_a={idx_a} idx_b={idx_b}")
        sys.exit(1)

    if idx_b >= idx_a:
        logger.info(
            f"FAIL: Packet B (vtime={VTIME_B_NS}) delivered at index {idx_b} "
            f"but Packet A (vtime={VTIME_A_NS}) delivered at index {idx_a}. "
            f"Expected B before A."
        )
        sys.exit(1)

    logger.info(f"PASS: Packet B delivered before Packet A [idx_b={idx_b} < idx_a={idx_a}]")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
