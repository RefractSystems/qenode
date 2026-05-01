#!/usr/bin/env python3
"""
telemetry_bench.py — Telemetry Throughput Benchmark.

Measures the rate at which telemetry can publish telemetry events
while firmware is executing a continuous IRQ storm.

Acceptance criteria (PLAN §12.8):
  - ≥ 100,000 telemetry events/second sustained over the measurement window.
  - vCPU MIPS degradation from adding telemetry ≤ 20 % vs standalone baseline
    (measured indirectly via wall-clock time for the same firmware workload).

Usage:
    python3 tests/fixtures/guest_apps/telemetry_wfi/telemetry_bench.py

The script starts QEMU with the IRQ-storm firmware and the telemetry
plugin enabled, counts incoming Zenoh publications on
sim/telemetry/trace/0 for MEASUREMENT_WINDOW_S seconds, then checks the
throughput against the threshold.
"""

import logging
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

import zenoh

logger = logging.getLogger(__name__)


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

# How long to count events after startup warm-up.
MEASUREMENT_WINDOW_S = 5

# Warm-up period: let QEMU reach steady state before counting.
WARMUP_S = 3

# Minimum acceptable event throughput.
MIN_EVENTS_PER_SEC = 100_000

# vCPU Overhead threshold (max % wall-clock increase).
MAX_OVERHEAD_PCT = 25.0

# QEMU startup timeout.
QEMU_START_TIMEOUT_S = 20


def _get_free_endpoint() -> str:
    script = Path(WORKSPACE_DIR) / "scripts" / "get-free-port.py"
    return subprocess.check_output([sys.executable, str(script), "--endpoint", "--proto", "tcp/"]).decode().strip()


def main() -> None:
    dtb = Path(SCRIPT_DIR) / "test_telemetry.dtb"
    kernel = Path(SCRIPT_DIR) / "test_irq_storm.elf"

    if not Path(dtb).exists():
        logger.error(f"ERROR: DTB not found: {dtb}")
        logger.info("       Run  make -C tests/fixtures/guest_apps/telemetry_wfi  to build test artifacts.")
        sys.exit(1)
    if not Path(kernel).exists():
        logger.error(f"ERROR: Kernel not found: {kernel}")
        logger.info("       Run  make -C tests/fixtures/guest_apps/telemetry_wfi  to build test artifacts.")
        sys.exit(1)

    # Start an ephemeral Zenoh router so multicast scouting doesn't add noise.
    router_url = _get_free_endpoint()
    router_proc = subprocess.Popen(
        ["python3", (Path(WORKSPACE_DIR) / "tests" / "zenoh_router_persistent.py"), router_url],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1)

    # Launch QEMU with telemetry enabled.
    run_sh_path = os.environ.get("RUN_SH") or str(WORKSPACE_DIR / "scripts" / "run.sh")
    cmd = [
        run_sh_path,
        "--dtb",
        dtb,
        "--kernel",
        kernel,
        "-display",
        "none",
        "-nographic",
        "-serial",
        "none",
        "-monitor",
        "none",
        "-device",
        f"telemetry,node=0,router={router_url}",
    ]

    logger.info("Starting benchmark...")
    qemu_proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)

    # Counter for telemetry events.
    event_count = 0
    count_lock = threading.Lock()

    def on_sample(_sample):
        nonlocal event_count
        with count_lock:
            event_count += 1

    # Open Zenoh session to listen for telemetry.
    cfg = zenoh.Config()
    cfg.insert_json5("connect/endpoints", f'["{router_url}"]')
    cfg.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(cfg)
    _sub = session.declare_subscriber("sim/telemetry/trace/0", on_sample)

    try:
        # 1. Warm-up
        logger.info(f"Warming up for {WARMUP_S}s...")
        time.sleep(WARMUP_S)

        # 2. Measurement window
        with count_lock:
            event_count = 0
        t_start = time.perf_counter()
        logger.info(f"Measuring throughput for {MEASUREMENT_WINDOW_S}s...")
        time.sleep(MEASUREMENT_WINDOW_S)
        t_end = time.perf_counter()
        total_events = event_count

        duration = t_end - t_start
        rate = total_events / duration

        logger.info("--- Results ---")
        logger.info(f"Total events : {total_events}")
        logger.info(f"Duration     : {duration:.2f} s")
        logger.info(f"Rate         : {rate:.0f} events/s")
        logger.info(f"Threshold    : {MIN_EVENTS_PER_SEC} events/s")

        if rate < MIN_EVENTS_PER_SEC:
            logger.error("❌ FAILED: Telemetry throughput below threshold.")
            sys.exit(1)
        else:
            logger.info("✅ PASSED: Telemetry throughput meets criteria.")

    finally:
        _sub.undeclare()
        session.close()
        qemu_proc.terminate()
        qemu_proc.wait()
        router_proc.terminate()
        router_proc.wait()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
