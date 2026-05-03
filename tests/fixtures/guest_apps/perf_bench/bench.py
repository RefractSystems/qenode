"""
SOTA Test Module: bench

Context:
This module implements tests for the bench subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of bench.
"""

import contextlib
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import typing
from pathlib import Path

import zenoh

from tools.testing.env import WORKSPACE_DIR
from tools.testing.utils import mock_execution_delay
from tools.vproto import ClockAdvanceReq, ClockReadyResp

logger = logging.getLogger(__name__)

# 10 ms quantums give ~30 RTT samples for the benchmark workload.
QUANTUM_NS = 10_000_000
MAX_QUANTUMS = 5000  # 50 s virtual cap
STANDALONE_TIMEOUT = 30

# IPS thresholds (PLAN §16.1). Values are MIPS; CI fails below the FAIL level.
MIPS_THRESHOLDS = {
    "standalone": {"warn": 80, "fail": 60},
    "slaved-icount": {"warn": 15, "fail": 10},
}

# Latency thresholds µs (PLAN §16.2). CI fails if either threshold is exceeded.
LATENCY_P50_FAIL_US = 10_000
LATENCY_P99_FAIL_US = 20_000

# ASan instrumentation significantly increases latency in the co-simulation loop.
if os.environ.get("VIRTMCU_USE_ASAN") == "1":
    LATENCY_P50_FAIL_US *= 3
    LATENCY_P99_FAIL_US *= 3

CNTFRQ_RE = re.compile(r"CNTFRQ:\s*([0-9a-fA-F]+)")
CYCLES_RE = re.compile(r"CYCLES:\s*([0-9a-fA-F]+)")


def _get_free_endpoint() -> str:
    script = Path(WORKSPACE_DIR) / "scripts" / "get-free-port.py"
    return subprocess.check_output([sys.executable, str(script), "--endpoint", "--proto", "tcp/"]).decode().strip()


def pack_req(delta_ns: int, quantum_number: int = 0) -> bytes:
    return ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=quantum_number).pack()


def unpack_rep(data: bytes) -> ClockReadyResp:
    return ClockReadyResp.unpack(data)


def _percentile(sorted_vals: list[float], p: float) -> float:
    if not sorted_vals:
        return 0.0
    idx = min(int(len(sorted_vals) * p / 100), len(sorted_vals) - 1)
    return sorted_vals[idx]


def latency_stats(latencies_ms: list[float]) -> str:
    if not latencies_ms:
        return "N/A"
    s = sorted(latencies_ms)
    mean = sum(s) / len(s)
    return (
        f"min={s[0]:.2f} mean={mean:.2f} "
        f"p95={_percentile(s, 95):.2f} p99={_percentile(s, 99):.2f} "
        f"max={s[-1]:.2f} ms  (n={len(s)})"
    )


class BenchmarkRunner:
    def __init__(self, mode: str, dtb: Path, kernel: Path, router: str) -> None:
        self.mode = mode
        self.dtb = dtb
        self.kernel = kernel
        self.router = router
        self.cntfrq = 0
        self.exit_cycles = 0
        self.exit_vtime_ns = 0
        self.wall_time = 0.0
        self.latencies: list[float] = []
        self.stall_count = 0
        self._exit_event = threading.Event()
        self._bench_done = False

    def _output_reader(self, proc: subprocess.Popen[str]) -> None:
        if not proc.stdout:
            return
        for line in proc.stdout:
            line_str = line.strip()
            logger.info(f"  [QEMU/{self.mode}/stdout] {line_str}")

            if not self.cntfrq:
                match = CNTFRQ_RE.search(line_str)
                if match:
                    with contextlib.suppress(ValueError):
                        self.cntfrq = int(match.group(1), 16)

            if not self.exit_cycles:
                match = CYCLES_RE.search(line_str)
                if match:
                    try:
                        self.exit_cycles = int(match.group(1), 16)
                    except ValueError as e:
                        logger.error(f"  [{self.mode}] CYCLES parse error: {e}")

            if "EXIT" in line_str:
                self._exit_event.set()

    def _stderr_relay(self, proc: subprocess.Popen[str]) -> None:
        if not proc.stderr:
            return
        for line in proc.stderr:
            logger.error(f"  [QEMU/{self.mode}/stderr] {line.strip()}")

    def _run_icount(self, proc: subprocess.Popen[str], t0: float) -> bool:
        config = zenoh.Config()
        config.insert_json5("connect/endpoints", f'["{self.router}"]')
        config.insert_json5("scouting/multicast/enabled", "false")
        logger.info(f"  [Test] Connecting to Zenoh router at {self.router}...")
        session = zenoh.open(config)

        topic = "sim/clock/advance/0"
        logger.info(f"  [Test] Waiting for queryable on {topic}...")

        ready = False
        deadline = time.perf_counter() + 15.0
        q_num = 0
        while time.perf_counter() < deadline:
            # Use a longer timeout for the ready check to allow QEMU to reach first boundary
            replies = list(session.get(topic, payload=pack_req(0, q_num), timeout=5.0))
            if replies:
                for r in replies:
                    if hasattr(r, "ok") and r.ok is not None:
                        ready = True
                        break
                    if hasattr(r, "err") and r.err is not None:
                        logger.info(f"  [Test] Reply error: {r.err}")
            if ready:
                break
            mock_execution_delay(0.2)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
            q_num += 1

        if not ready:
            logger.error(f"  ERROR: [{self.mode}] queryable not found after 15 s")
            typing.cast(typing.Any, session).close()
            self.wall_time = time.perf_counter() - t0
            return False

        current_q = q_num
        for q in range(MAX_QUANTUMS):
            if proc.poll() is not None:
                break

            lat0 = time.perf_counter()
            replies = list(session.get(topic, payload=pack_req(QUANTUM_NS, current_q), timeout=30.0))
            lat1 = time.perf_counter()
            current_q += 1

            if not replies or not hasattr(replies[0], "ok") or replies[0].ok is None:
                logger.error(f"  ERROR: [{self.mode}] quantum {q} — no reply")
                break

            resp = unpack_rep(replies[0].ok.payload.to_bytes())
            if resp.error_code != 0:
                logger.error(f"  ERROR: [{self.mode}] quantum {q} — error_code={resp.error_code}")
                if resp.error_code == 1:  # STALL
                    self.stall_count += 1
                break

            self.latencies.append((lat1 - lat0) * 1e3)

            if self._exit_event.is_set():
                # current_vtime_ns at quantum boundary after EXIT ≈ total instructions
                # (icount shift=0: 1 virtual ns = 1 instruction).
                self.exit_vtime_ns = resp.current_vtime_ns
                break
        else:
            logger.warning(f"  WARN: [{self.mode}] hit MAX_QUANTUMS ({MAX_QUANTUMS}) without EXIT")

        self.wall_time = time.perf_counter() - t0
        session.close()  # type: ignore[no-untyped-call]
        return True

    def run(self) -> None:
        run_sh_path = os.environ.get("RUN_SH") or str(WORKSPACE_DIR / "scripts" / "run.sh")
        retries = 3
        while retries > 0:
            self._exit_event.clear()
            self.exit_cycles = 0
            self.exit_vtime_ns = 0
            self.latencies = []
            self.stall_count = 0

            cmd = [
                run_sh_path,
                "--dtb",
                str(self.dtb),
                "--kernel",
                str(self.kernel),
                "-nographic",
                "-serial",
                "stdio",
                "-monitor",
                "none",
            ]
            if "slaved-icount" in self.mode:
                # Using slaved-suspend for benchmark as it's more stable
                # and still provides virtual-time slaving.
                cmd += [
                    "-icount",
                    "shift=0,align=off,sleep=off",
                    "-device",
                    f"virtmcu-clock,mode=slaved-suspend,node=0,router={self.router}",
                ]

            proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1,
            )
            threading.Thread(target=self._output_reader, args=(proc,), daemon=True).start()
            threading.Thread(target=self._stderr_relay, args=(proc,), daemon=True).start()

            t0 = time.perf_counter()
            if "slaved-icount" not in self.mode:
                deadline = t0 + STANDALONE_TIMEOUT
                while not self._exit_event.is_set() and proc.poll() is None:
                    if time.perf_counter() > deadline:
                        logger.error(f"  ERROR: [{self.mode}] timed out ({STANDALONE_TIMEOUT} s)")
                        break
                    mock_execution_delay(0.05)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
                self.wall_time = time.perf_counter() - t0
                success = True
            else:
                success = self._run_icount(proc, t0)

            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()

            if success:
                break
            retries -= 1
            if retries > 0:
                logger.info(f"  [{self.mode}] retrying… ({retries} left)")
                mock_execution_delay(2)  # SLEEP_EXCEPTION: mock test simulating execution/spacing


def main() -> None:
    dtb = Path(__file__).resolve().parent / "minimal.dtb"
    kernel = Path(__file__).resolve().parent / "bench.elf"

    try:
        subprocess.run(
            [
                shutil.which("dtc") or "dtc",
                "-I",
                "dts",
                "-O",
                "dtb",
                "-o",
                str(dtb),
                str(Path(WORKSPACE_DIR) / "tests/fixtures/guest_apps/boot_arm/minimal.dts"),
            ],
            check=True,
            capture_output=True,
        )
    except subprocess.CalledProcessError as e:
        logger.error(f"Failed to compile dtb: {e.stderr.decode()}")
        sys.exit(1)

    zenoh_router = _get_free_endpoint()
    router = subprocess.Popen(
        [
            shutil.which("python3") or "python3",
            str(Path(WORKSPACE_DIR) / "tests" / "zenoh_router_persistent.py"),
            zenoh_router,
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    from tools.testing.utils import wait_for_zenoh_router

    if not wait_for_zenoh_router(zenoh_router):
        router.kill()
        sys.exit(1)

    results = {}
    try:
        for mode in ("standalone", "slaved-icount", "slaved-icount-2"):
            logger.info(f"--- [{mode}] ---")
            runner = BenchmarkRunner(mode, dtb, kernel, zenoh_router)
            runner.run()
            results[mode] = runner
            logger.info(f"  wall  : {runner.wall_time:.3f} s")
            if runner.cntfrq:
                logger.info(f"  cntfrq: {runner.cntfrq:,} Hz  (NOTE: QEMU counter increments at 1 GHz regardless)")
            if runner.exit_cycles:
                logger.info(f"  cycles: {runner.exit_cycles:,}")
            if runner.exit_vtime_ns:
                logger.info(
                    f"  vtime : {runner.exit_vtime_ns / 1e6:.3f} ms virtual "
                    f"({runner.exit_vtime_ns:,} ns ≈ instructions)"
                )
            if runner.latencies:
                logger.info(f"  rtt   : {latency_stats(runner.latencies)}")
    finally:
        router.terminate()
        router.wait()

    logger.info("\n=== Performance Summary ===")

    r_sa = results["standalone"]
    r_ic = results["slaved-icount"]
    r_ic2 = results["slaved-icount-2"]

    if not r_ic.exit_cycles:
        logger.error("ERROR: slaved-icount run produced no CYCLES output")
        sys.exit(1)

    # Determinism: firmware CNTVCT delta must be identical across runs.
    drift_threshold = 0
    if abs(r_ic.exit_cycles - r_ic2.exit_cycles) <= drift_threshold:
        logger.info(f"Determinism          : PASSED  ({r_ic.exit_cycles:,} vs {r_ic2.exit_cycles:,} cycles)")
    else:
        diff = abs(r_ic.exit_cycles - r_ic2.exit_cycles)
        logger.error(f"Determinism          : FAILED  (delta={diff} cycles)")
        sys.exit(1)

    # IPS: use Zenoh vtime (icount shift=0 → vtime_ns == instructions).
    failures = []
    json_results = []

    mips_ic = 0.0
    if r_ic.exit_vtime_ns and r_ic.wall_time > 0:
        mips_ic = r_ic.exit_vtime_ns / r_ic.wall_time / 1e6
        logger.info(f"slaved-icount MIPS   : {mips_ic:.1f}")
        record = {"mode": "slaved-icount", "mips": round(mips_ic, 1)}
        json_results.append(record)
        logger.info(json.dumps(record))
        thresh = MIPS_THRESHOLDS.get("slaved-icount")
        if thresh and mips_ic < thresh["fail"]:
            failures.append(f"slaved-icount MIPS {mips_ic:.1f} < fail threshold {thresh['fail']}")

    mips_sa = 0.0
    if r_sa.exit_cycles and r_sa.wall_time > 0 and r_ic.exit_vtime_ns:
        mips_sa = r_ic.exit_vtime_ns / r_sa.wall_time / 1e6
        logger.info(f"standalone MIPS (est): {mips_sa:.1f}")
        record = {"mode": "standalone", "mips": round(mips_sa, 1)}
        json_results.append(record)
        logger.info(json.dumps(record))
        thresh = MIPS_THRESHOLDS.get("standalone")
        if thresh and mips_sa < thresh["fail"]:
            failures.append(f"standalone MIPS {mips_sa:.1f} < fail threshold {thresh['fail']}")

    # Latency thresholds (PLAN §16.2).
    if r_ic.latencies:
        logger.info(f"Co-sim latency       : {latency_stats(r_ic.latencies)}")
        sorted_lat = sorted(r_ic.latencies)
        p50_us = _percentile(sorted_lat, 50) * 1_000
        p99_us = _percentile(sorted_lat, 99) * 1_000
        stall_count = r_ic.stall_count + r_ic2.stall_count
        latency_record = {
            "p50_us": round(p50_us, 1),
            "p99_us": round(p99_us, 1),
            "stalls": stall_count,
        }
        json_results.append(latency_record)  # type: ignore[arg-type]
        logger.info(json.dumps(latency_record))
        if p50_us > LATENCY_P50_FAIL_US:
            failures.append(f"P50 latency {p50_us:.0f} µs > fail threshold {LATENCY_P50_FAIL_US} µs")
        if p99_us > LATENCY_P99_FAIL_US:
            failures.append(f"P99 latency {p99_us:.0f} µs > fail threshold {LATENCY_P99_FAIL_US} µs")
        if stall_count > 0:
            failures.append(f"clock stalls detected: {stall_count} (must be 0)")

    # Persist results for trend tracking ().
    results_path = Path(__file__).resolve().parent / "last_results.json"
    with Path(results_path).open("w") as f:
        json.dump(json_results, f, indent=2)

    if failures:
        for msg in failures:
            logger.error(f"THRESHOLD FAILURE: {msg}")
        if os.environ.get("VIRTMCU_USE_ASAN") == "1":
            logger.warning("WARNING: Bypassing performance failures because ASan is active.")
        else:
            sys.exit(1)

    logger.info("=== PASSED ===")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
