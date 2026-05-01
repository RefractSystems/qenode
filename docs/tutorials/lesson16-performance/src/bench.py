import json
import logging
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

import zenoh


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent  # Fallback

SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE_DIR = _find_workspace_root(Path(__file__).resolve())
TOOLS_DIR = WORKSPACE_DIR / "tools"

if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import contextlib  # noqa: E402

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

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


def _get_free_endpoint() -> str:
    script = Path(WORKSPACE_DIR) / "scripts" / "get-free-port.py"
    return subprocess.check_output([sys.executable, str(script), "--endpoint", "--proto", "tcp/"]).decode().strip()


def pack_req(delta_ns, quantum_number=0):
    return ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=quantum_number).pack()


def unpack_rep(data):
    return ClockReadyResp.unpack(data)


def _percentile(sorted_vals, p):
    idx = min(int(len(sorted_vals) * p / 100), len(sorted_vals) - 1)
    return sorted_vals[idx]


def latency_stats(latencies_ms):
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
    def __init__(self, mode, dtb, kernel, router):
        self.mode = mode
        self.dtb = dtb
        self.kernel = kernel
        self.router = router
        self.cntfrq = 0
        self.exit_cycles = 0
        self.exit_vtime_ns = 0
        self.wall_time = 0
        self.latencies = []
        self.stall_count = 0
        self._exit_event = threading.Event()
        self._bench_done = False

    def _output_reader(self, proc):
        for line in proc.stdout:
            logger.info(f"  [QEMU/{self.mode}/stdout] {line.strip()}")
            if "CNTFRQ: " in line and not self.cntfrq:
                with contextlib.suppress(Exception):
                    self.cntfrq = int(line.split("CNTFRQ: ")[1].strip(), 16)
            if "CYCLES: " in line and not self.exit_cycles:
                try:
                    self.exit_cycles = int(line.split("CYCLES: ")[1].strip(), 16)
                except Exception as e:
                    logger.info(f"  [{self.mode}] CYCLES parse error: {e}")
            if "EXIT" in line:
                self._exit_event.set()

    def _stderr_relay(self, proc):
        for line in proc.stderr:
            logger.error(f"  [QEMU/{self.mode}/stderr] {line.strip()}")

    def _run_icount(self, proc, t0) -> bool:
        config = zenoh.Config()
        config.insert_json5("connect/endpoints", f'["{self.router}"]')
        config.insert_json5("scouting/multicast/enabled", "false")
        logger.info(f"  [Test] Connecting to Zenoh router at {self.router}...")
        session = zenoh.open(config)

        topic = "sim/clock/advance/0"
        logger.info(f"  [Test] Waiting for queryable on {topic}...")

        ready = False
        deadline = time.perf_counter() + 15
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
            time.sleep(0.2)
            q_num += 1

        if not ready:
            logger.info(f"  ERROR: [{self.mode}] queryable not found after 15 s")
            session.close()
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
                logger.info(f"  ERROR: [{self.mode}] quantum {q} — no reply")
                break

            resp = unpack_rep(replies[0].ok.payload.to_bytes())
            if resp.error_code != 0:
                logger.info(f"  ERROR: [{self.mode}] quantum {q} — error_code={resp.error_code}")
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
            logger.info(f"  WARN: [{self.mode}] hit MAX_QUANTUMS ({MAX_QUANTUMS}) without EXIT")

        self.wall_time = time.perf_counter() - t0
        session.close()
        return True

    def run(self):
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
                self.dtb,
                "--kernel",
                self.kernel,
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
                        logger.info(f"  ERROR: [{self.mode}] timed out ({STANDALONE_TIMEOUT} s)")
                        break
                    time.sleep(0.05)
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
                time.sleep(2)


def main():
    dtb = Path(Path(SCRIPT_DIR).parent) / "minimal.dtb"
    kernel = Path(Path(SCRIPT_DIR).parent) / "bench.elf"

    subprocess.run(
        ["dtc", "-I", "dts", "-O", "dtb", "-o", dtb, (Path(SCRIPT_DIR) / "minimal.dts")],
        check=True,
        capture_output=True,
    )

    zenoh_router = _get_free_endpoint()
    router = subprocess.Popen(
        ["python3", (Path(WORKSPACE_DIR) / "tests" / "zenoh_router_persistent.py"), zenoh_router],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(2)

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
        logger.info(f"Determinism          : FAILED  (delta={diff} cycles)")
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
        json_results.append(latency_record)
        logger.info(json.dumps(latency_record))
        if p50_us > LATENCY_P50_FAIL_US:
            failures.append(f"P50 latency {p50_us:.0f} µs > fail threshold {LATENCY_P50_FAIL_US} µs")
        if p99_us > LATENCY_P99_FAIL_US:
            failures.append(f"P99 latency {p99_us:.0f} µs > fail threshold {LATENCY_P99_FAIL_US} µs")
        if stall_count > 0:
            failures.append(f"clock stalls detected: {stall_count} (must be 0)")

    # Persist results for trend tracking (Performance Benchmark).
    results_path = Path(SCRIPT_DIR) / "last_results.json"
    with Path(results_path).open("w") as f:
        json.dump(json_results, f, indent=2)

    if failures:
        for msg in failures:
            logger.info(f"THRESHOLD FAILURE: {msg}")
        sys.exit(1)

    logger.info("=== Performance Benchmark PASSED ===")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
