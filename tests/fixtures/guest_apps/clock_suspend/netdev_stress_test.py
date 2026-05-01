"""
SOTA Test Module: netdev_stress_test

Context:
This module implements tests for the netdev_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of netdev_stress_test.
"""

import logging
import os
import subprocess
import sys
import time
from pathlib import Path

import vproto
import zenoh

logger = logging.getLogger(__name__)

WORKSPACE_DIR = "/workspace"


def pack_zenoh_frame(vtime_ns: int, data: bytes) -> bytes:
    header = vproto.ZenohFrameHeader(vtime_ns, 0, len(data).pack())
    return header + data


def main():
    logger.info("Starting Zenoh router...")
    port_script = Path(WORKSPACE_DIR) / "scripts" / "get-free-port.py"
    router_endpoint = subprocess.check_output([sys.executable, str(port_script), "--endpoint", "--proto", "tcp/"]).decode().strip()
    router_proc = subprocess.Popen(
        [sys.executable, str(Path(WORKSPACE_DIR) / "tests" / "zenoh_router_persistent.py"), router_endpoint]
    )
    time.sleep(2)

    logger.info("Starting QEMU...")
    run_sh_path = os.environ.get("RUN_SH") or str(Path(WORKSPACE_DIR) / "scripts" / "run.sh")
    qemu_cmd = [
        run_sh_path,
        "--dtb",
        Path(WORKSPACE_DIR) / "test-results" / "netdev_determinism" / "board.dtb",
        "-kernel",
        Path(WORKSPACE_DIR) / "test-results" / "netdev_determinism" / "firmware.elf",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-netdev",
        "zenoh,id=net0,node=0,router=" + router_endpoint,
        "-nographic",
        "-monitor",
        "none",
    ]

    qemu_proc = subprocess.Popen(qemu_cmd, stderr=subprocess.PIPE, text=True)

    # Wait for QEMU to boot and subscribe to the topic
    time.sleep(3)

    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", f'["{router_endpoint}"]')
    conf.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(conf)

    rx_topic = "sim/eth/frame/0/rx"

    logger.info("Injecting 1000 packets out of order...")
    base_time = 1_000_000_000  # 1 second in ns
    for i in range(1000):
        # Reverse order: first packet sent has the largest vtime
        vtime = base_time + (1000 - i) * 1000
        data = f"PACKET_{i}".encode()
        session.put(rx_topic, pack_zenoh_frame(vtime, data))

    logger.info("Waiting for deliveries...")
    delivered_vtimes = []
    deadline = time.time() + 15.0
    while time.time() < deadline:
        line = qemu_proc.stderr.readline()
        if not line:
            break
        if "[virtmcu-netdev] RX deliver" in line:
            parts = line.split()
            vtime_str = next(p for p in parts if p.startswith("vtime="))
            vtime = int(vtime_str.split("=")[1])
            delivered_vtimes.append(vtime)
            if len(delivered_vtimes) == 1000:
                break

    qemu_proc.terminate()
    qemu_proc.wait()
    router_proc.terminate()
    router_proc.wait()

    if len(delivered_vtimes) != 1000:
        logger.error(f"FAIL: Only delivered {len(delivered_vtimes)}/1000 packets.")
        sys.exit(1)

    if delivered_vtimes == sorted(delivered_vtimes):
        logger.info(
            "PASS: 1000 packets delivered in perfect virtual-time order despite being injected in reverse order!"
        )
        sys.exit(0)
    else:
        logger.error("FAIL: Packets delivered out of order!")
        sys.exit(1)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
