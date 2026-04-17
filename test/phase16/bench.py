import os
import sys
import time
import subprocess
import threading
import zenoh

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_DIR = os.path.dirname(os.path.dirname(SCRIPT_DIR))
sys.path.append(os.path.join(WORKSPACE_DIR, "tools"))

from vproto import ClockAdvanceReq, ClockReadyResp

# 1 ms quantums: fine-grained latency samples, firmware (~40 ms virtual) fits ~40 quantums
QUANTUM_NS = 1_000_000
MAX_QUANTUMS = 500   # 500 ms virtual cap; firmware exits well before this
ZENOH_ROUTER = "tcp/127.0.0.1:7447"
STANDALONE_TIMEOUT = 30


def pack_req(delta_ns):
    return ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0).pack()


def unpack_rep(data):
    return ClockReadyResp.unpack(data)


def _percentile(sorted_vals, p):
    idx = int(len(sorted_vals) * p / 100)
    idx = min(idx, len(sorted_vals) - 1)
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
    def __init__(self, mode, dtb, kernel):
        self.mode = mode
        self.dtb = dtb
        self.kernel = kernel
        self._exit_event = threading.Event()
        self.exit_vtime_ns = 0
        self.wall_time = 0.0
        self.latencies = []  # round-trip ms per quantum

    def _output_reader(self, proc):
        for line in proc.stdout:
            if "EXIT" in line:
                self._exit_event.set()

    def run(self):
        run_sh = os.path.join(WORKSPACE_DIR, "scripts", "run.sh")
        cmd = [run_sh, "--dtb", self.dtb, "--kernel", self.kernel,
               "-nographic", "-serial", "stdio", "-monitor", "none"]

        if "slaved-icount" in self.mode:
            cmd += [
                "-icount", "shift=0,align=off,sleep=off",
                "-device", f"zenoh-clock,mode=icount,node=0,router={ZENOH_ROUTER}",
            ]

        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, bufsize=1,
        )

        reader = threading.Thread(
            target=self._output_reader, args=(proc,), daemon=True
        )
        reader.start()

        t0 = time.perf_counter()

        if "slaved-icount" not in self.mode:
            deadline = t0 + STANDALONE_TIMEOUT
            while not self._exit_event.is_set() and proc.poll() is None:
                if time.perf_counter() > deadline:
                    break
                time.sleep(0.05)
            self.wall_time = time.perf_counter() - t0
        else:
            self._run_icount(proc, t0)

        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()

    def _run_icount(self, proc, t0):
        config = zenoh.Config()
        config.insert_json5("connect/endpoints", f'["{ZENOH_ROUTER}"]')
        config.insert_json5("scouting/multicast/enabled", "false")
        session = zenoh.open(config)

        topic = "sim/clock/advance/0"

        # Wait for queryable (QEMU zenoh-clock registers it shortly after boot)
        ready = False
        deadline = time.perf_counter() + 15
        while time.perf_counter() < deadline:
            r = list(session.get(topic, payload=pack_req(0), timeout=1.0))
            if r and r[0].ok:
                ready = True
                break
            time.sleep(0.2)

        if not ready:
            print(f"  ERROR: [{self.mode}] queryable not found after 15 s")
            session.close()
            self.wall_time = time.perf_counter() - t0
            return

        for q in range(MAX_QUANTUMS):
            if proc.poll() is not None:
                break

            lat0 = time.perf_counter()
            replies = list(session.get(topic, payload=pack_req(QUANTUM_NS), timeout=30.0))
            lat1 = time.perf_counter()

            if not replies or not replies[0].ok:
                print(f"  ERROR: [{self.mode}] quantum {q} — no reply")
                break

            resp = unpack_rep(replies[0].ok.payload.to_bytes())
            if resp.error_code != 0:
                print(f"  ERROR: [{self.mode}] quantum {q} — error_code={resp.error_code}")
                break

            self.latencies.append((lat1 - lat0) * 1e3)

            if self._exit_event.is_set():
                # Record vtime at the quantum boundary right after EXIT was printed.
                # With icount shift=0, current_vtime_ns ≈ total instructions executed.
                self.exit_vtime_ns = resp.current_vtime_ns
                break
        else:
            print(f"  WARN: [{self.mode}] hit MAX_QUANTUMS ({MAX_QUANTUMS}) without EXIT")

        self.wall_time = time.perf_counter() - t0
        session.close()


def main():
    dtb = os.path.join(SCRIPT_DIR, "minimal.dtb")
    dts = os.path.join(WORKSPACE_DIR, "test/phase1/minimal.dts")
    kernel = os.path.join(SCRIPT_DIR, "bench.elf")

    subprocess.run(
        ["dtc", "-I", "dts", "-O", "dtb", "-o", dtb, dts],
        check=True, capture_output=True,
    )

    router = subprocess.Popen(
        ["python3", os.path.join(WORKSPACE_DIR, "tests", "zenoh_router_persistent.py")],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(2)

    results = {}
    try:
        for mode in ("standalone", "slaved-icount", "slaved-icount-2"):
            print(f"--- [{mode}] ---")
            runner = BenchmarkRunner(mode, dtb, kernel)
            runner.run()
            results[mode] = runner
            print(f"  wall : {runner.wall_time:.3f} s")
            if runner.exit_vtime_ns:
                print(f"  vtime: {runner.exit_vtime_ns / 1e9:.4f} s  ({runner.exit_vtime_ns:,} ns)")
            if runner.latencies:
                print(f"  rtt  : {latency_stats(runner.latencies)}")
    finally:
        router.terminate()
        router.wait()

    print("\n=== Performance Summary ===")

    r_sa = results["standalone"]
    r_ic = results["slaved-icount"]
    r_ic2 = results["slaved-icount-2"]

    # Instruction count proxy: icount shift=0 means 1 virtual ns = 1 instruction
    instr = r_ic.exit_vtime_ns
    if instr == 0:
        print("ERROR: slaved-icount run did not detect EXIT — no instruction count available")
        sys.exit(1)

    print(f"Instructions (proxy) : {instr:,}")

    # Determinism: both icount runs must finish at the exact same virtual time
    if r_ic.exit_vtime_ns > 0 and r_ic.exit_vtime_ns == r_ic2.exit_vtime_ns:
        print("Determinism          : PASSED")
    else:
        delta = abs(r_ic.exit_vtime_ns - r_ic2.exit_vtime_ns)
        print(f"Determinism          : FAILED (delta={delta} ns)")
        sys.exit(1)

    # IPS
    if r_ic.wall_time > 0:
        mips = instr / r_ic.wall_time / 1e6
        print(f"slaved-icount MIPS   : {mips:.1f}")
    if r_sa.wall_time > 0 and r_sa._exit_event.is_set():
        mips_sa = instr / r_sa.wall_time / 1e6
        print(f"standalone MIPS (est): {mips_sa:.1f}")

    # Latency
    if r_ic.latencies:
        print(f"Co-sim latency       : {latency_stats(r_ic.latencies)}")

    print("=== Phase 16 PASSED ===")


if __name__ == "__main__":
    main()
