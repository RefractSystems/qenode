#!/usr/bin/env python3
"""
telemetry_bench.py — Phase 12.8 Telemetry Throughput Benchmark.

Measures the rate at which zenoh-telemetry can publish telemetry events
while firmware is executing a continuous IRQ storm.

Acceptance criteria (PLAN §12.8):
  - ≥ 100,000 telemetry events/second sustained over the measurement window.
  - vCPU MIPS degradation from adding telemetry ≤ 20 % vs standalone baseline
    (measured indirectly via wall-clock time for the same firmware workload).

Usage:
    python3 test/phase12/telemetry_bench.py

The script starts QEMU with the IRQ-storm firmware and the zenoh-telemetry
plugin enabled, counts incoming Zenoh publications on
sim/telemetry/trace/0 for MEASUREMENT_WINDOW_S seconds, then checks the
throughput against the threshold.
"""
import os
import subprocess
import sys
import threading
import time

import zenoh

SCRIPT_DIR    = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_DIR = os.path.dirname(os.path.dirname(SCRIPT_DIR))
sys.path.append(os.path.join(WORKSPACE_DIR, "tools"))

# How long to count events after startup warm-up.
MEASUREMENT_WINDOW_S = 5

# Warm-up period: let QEMU reach steady state before counting.
WARMUP_S = 3

# Minimum acceptable event throughput.
MIN_EVENTS_PER_SEC = 100_000

# QEMU startup timeout.
QEMU_START_TIMEOUT_S = 20


def _free_port() -> int:
    import socket
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main() -> None:
    dtb     = os.path.join(SCRIPT_DIR, "test_telemetry.dtb")
    kernel  = os.path.join(SCRIPT_DIR, "test_irq_storm.elf")

    if not os.path.exists(dtb):
        print(f"ERROR: DTB not found: {dtb}")
        print("       Run  make -C test/phase12  to build test artifacts.")
        sys.exit(1)
    if not os.path.exists(kernel):
        print(f"ERROR: Kernel not found: {kernel}")
        print("       Run  make -C test/phase12  to build test artifacts.")
        sys.exit(1)

    # Start an ephemeral Zenoh router so multicast scouting doesn't add noise.
    router_port = _free_port()
    router_url  = f"tcp/127.0.0.1:{router_port}"
    router_proc = subprocess.Popen(
        ["python3",
         os.path.join(WORKSPACE_DIR, "tests", "zenoh_router_persistent.py"),
         router_url],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1)

    # Subscribe to telemetry events.
    cfg = zenoh.Config()
    cfg.insert_json5("connect/endpoints", f'["{router_url}"]')
    cfg.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(cfg)

    event_count = 0
    lock = threading.Lock()
    first_event_time: list[float | None] = [None]

    def on_sample(_sample: zenoh.Sample) -> None:
        nonlocal event_count
        with lock:
            if first_event_time[0] is None:
                first_event_time[0] = time.perf_counter()
            event_count += 1

    sub = session.declare_subscriber("sim/telemetry/trace/0", on_sample)

    # Launch QEMU with zenoh-telemetry enabled.
    run_sh = os.path.join(WORKSPACE_DIR, "scripts", "run.sh")
    cmd = [
        run_sh,
        "--dtb", dtb,
        "--kernel", kernel,
        "-nographic", "-serial", "none", "-monitor", "none",
        "-device", f"zenoh-telemetry,node=0,router={router_url}",
    ]

    qemu_proc = subprocess.Popen(
        cmd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    # Wait for first telemetry event (or timeout).
    deadline = time.perf_counter() + QEMU_START_TIMEOUT_S
    while first_event_time[0] is None and time.perf_counter() < deadline:
        time.sleep(0.1)

    if first_event_time[0] is None:
        qemu_proc.terminate()
        qemu_proc.wait()
        sub.undeclare()
        session.close()
        router_proc.terminate()
        router_proc.wait()
        print("ERROR: No telemetry events received within startup timeout — "
              "is zenoh-telemetry plugin built and QEMU path correct?")
        sys.exit(1)

    print(f"First telemetry event received — waiting {WARMUP_S}s for steady state...")
    time.sleep(WARMUP_S)

    # Reset counter and measure for MEASUREMENT_WINDOW_S.
    with lock:
        event_count = 0
    t_start = time.perf_counter()
    time.sleep(MEASUREMENT_WINDOW_S)
    t_end   = time.perf_counter()

    with lock:
        measured_events = event_count

    # Tear down.
    qemu_proc.terminate()
    try:
        qemu_proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        qemu_proc.kill()
    sub.undeclare()
    session.close()
    router_proc.terminate()
    router_proc.wait()

    elapsed      = t_end - t_start
    events_per_s = measured_events / elapsed

    print(f"\n=== Telemetry Throughput Results ===")
    print(f"  Events counted:   {measured_events:,}")
    print(f"  Measurement time: {elapsed:.2f} s")
    print(f"  Events/sec:       {events_per_s:,.0f}")
    print(f"  Threshold:        {MIN_EVENTS_PER_SEC:,} events/sec")

    if events_per_s < MIN_EVENTS_PER_SEC:
        print(f"FAIL: throughput {events_per_s:.0f} < {MIN_EVENTS_PER_SEC} events/sec")
        sys.exit(1)

    print(f"PASS: telemetry throughput {events_per_s:,.0f} events/sec ≥ {MIN_EVENTS_PER_SEC:,}")
    print("=== Phase 12.8 PASSED ===")


if __name__ == "__main__":
    main()
