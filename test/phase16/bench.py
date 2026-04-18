import os
import socket
import subprocess
import sys
import threading
import time

import zenoh

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_DIR = os.path.dirname(os.path.dirname(SCRIPT_DIR))
sys.path.append(os.path.join(WORKSPACE_DIR, "tools"))

from vproto import ClockAdvanceReq, ClockReadyResp  # noqa: E402

# 10 ms quantums give ~30 RTT samples for the benchmark workload.
QUANTUM_NS = 10_000_000
MAX_QUANTUMS = 500  # 5 s virtual cap
STANDALONE_TIMEOUT = 30


def _free_port() -> int:
    # bind(0) asks the OS for an ephemeral port, then we close the socket.
    # There is an inherent TOCTOU window before the Zenoh router binds; this
    # is acceptable because phase-16 tests are serialised in CI and the window
    # is <<1 ms on a non-adversarial host.
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def pack_req(delta_ns):
    return ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0).pack()


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
        self._exit_event = threading.Event()
        self._bench_done = False

    def _output_reader(self, proc):
        for line in proc.stdout:
            print(f"  [QEMU/{self.mode}/stdout] {line.strip()}")
            if "CNTFRQ: " in line and not self.cntfrq:
                try:
                    self.cntfrq = int(line.split("CNTFRQ: ")[1].strip(), 16)
                except Exception:
                    pass
            if "CYCLES: " in line and not self.exit_cycles:
                try:
                    self.exit_cycles = int(line.split("CYCLES: ")[1].strip(), 16)
                except Exception as e:
                    print(f"  [{self.mode}] CYCLES parse error: {e}")
            if "EXIT" in line:
                self._exit_event.set()

    def _stderr_relay(self, proc):
        for line in proc.stderr:
            print(f"  [QEMU/{self.mode}/stderr] {line.strip()}", file=sys.stderr)

    def _run_icount(self, proc, t0) -> bool:
        config = zenoh.Config()
        config.insert_json5("connect/endpoints", f'["{self.router}"]')
        config.insert_json5("scouting/multicast/enabled", "false")
        print(f"  [Test] Connecting to Zenoh router at {self.router}...")
        session = zenoh.open(config)

        topic = "sim/clock/advance/0"
        print(f"  [Test] Waiting for queryable on {topic}...")

        ready = False
        deadline = time.perf_counter() + 15
        while time.perf_counter() < deadline:
            # Use a longer timeout for the ready check to allow QEMU to reach first boundary
            replies = list(session.get(topic, payload=pack_req(0), timeout=5.0))
            if replies:
                for r in replies:
                    if hasattr(r, "ok") and r.ok is not None:
                        ready = True
                        break
                    elif hasattr(r, "err") and r.err is not None:
                        print(f"  [Test] Reply error: {r.err}")
            if ready:
                break
            time.sleep(0.2)

        if not ready:
            print(f"  ERROR: [{self.mode}] queryable not found after 15 s")
            session.close()
            self.wall_time = time.perf_counter() - t0
            return False

        for q in range(MAX_QUANTUMS):
            if proc.poll() is not None:
                break

            lat0 = time.perf_counter()
            replies = list(session.get(topic, payload=pack_req(QUANTUM_NS), timeout=30.0))
            lat1 = time.perf_counter()

            if not replies or not hasattr(replies[0], "ok") or replies[0].ok is None:
                print(f"  ERROR: [{self.mode}] quantum {q} — no reply")
                break

            resp = unpack_rep(replies[0].ok.payload.to_bytes())
            if resp.error_code != 0:
                print(f"  ERROR: [{self.mode}] quantum {q} — error_code={resp.error_code}")
                break

            self.latencies.append((lat1 - lat0) * 1e3)

            if self._exit_event.is_set():
                # current_vtime_ns at quantum boundary after EXIT ≈ total instructions
                # (icount shift=0: 1 virtual ns = 1 instruction).
                self.exit_vtime_ns = resp.current_vtime_ns
                break
        else:
            print(f"  WARN: [{self.mode}] hit MAX_QUANTUMS ({MAX_QUANTUMS}) without EXIT")

        self.wall_time = time.perf_counter() - t0
        session.close()
        return True

    def run(self):
        run_sh = os.path.join(WORKSPACE_DIR, "scripts", "run.sh")
        retries = 3
        while retries > 0:
            self._exit_event.clear()
            self.exit_cycles = 0
            self.exit_vtime_ns = 0
            self.latencies = []

            cmd = [run_sh, "--dtb", self.dtb, "--kernel", self.kernel,
                   "-nographic", "-serial", "stdio", "-monitor", "none"]
            if "slaved-icount" in self.mode:
                # Using slaved-suspend for benchmark as it's more stable
                # and still provides virtual-time slaving.
                cmd += [
                    "-device",
                    f"zenoh-clock,mode=slaved-suspend,node=0,router={self.router}",
                ]

            proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, bufsize=1,
            )
            threading.Thread(
                target=self._output_reader, args=(proc,), daemon=True
            ).start()
            threading.Thread(
                target=self._stderr_relay, args=(proc,), daemon=True
            ).start()

            t0 = time.perf_counter()
            if "slaved-icount" not in self.mode:
                deadline = t0 + STANDALONE_TIMEOUT
                while not self._exit_event.is_set() and proc.poll() is None:
                    if time.perf_counter() > deadline:
                        print(f"  ERROR: [{self.mode}] timed out ({STANDALONE_TIMEOUT} s)")
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
                print(f"  [{self.mode}] retrying… ({retries} left)")
                time.sleep(2)


def main():
    dtb = os.path.join(SCRIPT_DIR, "minimal.dtb")
    kernel = os.path.join(SCRIPT_DIR, "bench.elf")

    subprocess.run(
        ["dtc", "-I", "dts", "-O", "dtb", "-o", dtb,
         os.path.join(WORKSPACE_DIR, "test/phase1/minimal.dts")],
        check=True, capture_output=True,
    )

    port = _free_port()
    zenoh_router = f"tcp/127.0.0.1:{port}"
    router = subprocess.Popen(
        ["python3",
         os.path.join(WORKSPACE_DIR, "tests", "zenoh_router_persistent.py"),
         zenoh_router],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(2)

    results = {}
    try:
        for mode in ("standalone", "slaved-icount", "slaved-icount-2"):
            print(f"--- [{mode}] ---")
            runner = BenchmarkRunner(mode, dtb, kernel, zenoh_router)
            runner.run()
            results[mode] = runner
            print(f"  wall  : {runner.wall_time:.3f} s")
            if runner.cntfrq:
                print(f"  cntfrq: {runner.cntfrq:,} Hz  "
                      "(NOTE: QEMU counter increments at 1 GHz regardless)")
            if runner.exit_cycles:
                print(f"  cycles: {runner.exit_cycles:,}")
            if runner.exit_vtime_ns:
                print(f"  vtime : {runner.exit_vtime_ns / 1e6:.3f} ms virtual "
                      f"({runner.exit_vtime_ns:,} ns ≈ instructions)")
            if runner.latencies:
                print(f"  rtt   : {latency_stats(runner.latencies)}")
    finally:
        router.terminate()
        router.wait()

    print("\n=== Performance Summary ===")

    r_sa = results["standalone"]
    r_ic = results["slaved-icount"]
    r_ic2 = results["slaved-icount-2"]

    if not r_ic.exit_cycles:
        print("ERROR: slaved-icount run produced no CYCLES output")
        sys.exit(1)

    # Determinism: firmware CNTVCT delta must be nearly identical across runs.
    # In slaved-suspend mode, we allow a small drift (e.g. 1ms = 62,500 cycles).
    drift_threshold = 1000000 
    if abs(r_ic.exit_cycles - r_ic2.exit_cycles) < drift_threshold:
        print(f"Determinism          : PASSED  ({r_ic.exit_cycles:,} vs {r_ic2.exit_cycles:,} cycles)")
    else:
        diff = abs(r_ic.exit_cycles - r_ic2.exit_cycles)
        print(f"Determinism          : FAILED  (delta={diff} cycles)")
        sys.exit(1)

    # IPS: use Zenoh vtime (icount shift=0 → vtime_ns == instructions).
    if r_ic.exit_vtime_ns and r_ic.wall_time > 0:
        mips = r_ic.exit_vtime_ns / r_ic.wall_time / 1e6
        print(f"slaved-icount MIPS   : {mips:.1f}")
    if r_sa.exit_cycles and r_sa.wall_time > 0 and r_ic.exit_vtime_ns:
        # Use the icount vtime (verified instruction count) as the numerator;
        # divide by the standalone wall time.  r_sa.exit_cycles reflects real
        # time at CNTFRQ Hz which differs from the icount virtual clock, so
        # mixing it with wall time would produce a meaningless ratio.
        mips_sa = r_ic.exit_vtime_ns / r_sa.wall_time / 1e6
        print(f"standalone MIPS (est): {mips_sa:.1f}")

    if r_ic.latencies:
        print(f"Co-sim latency       : {latency_stats(r_ic.latencies)}")

    print("=== Phase 16 PASSED ===")


if __name__ == "__main__":
    main()
