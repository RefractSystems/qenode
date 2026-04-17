import os
import sys
import time
import subprocess
import threading
import zenoh
import struct

# Add tools/ to path
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_DIR = os.path.dirname(os.path.dirname(SCRIPT_DIR))
sys.path.append(os.path.join(WORKSPACE_DIR, "tools"))

from vproto import ClockAdvanceReq, ClockReadyResp

def pack_req(delta_ns):
    return ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0).pack()

def unpack_rep(data):
    return ClockReadyResp.unpack(data)

class BenchmarkRunner:
    def __init__(self, mode, dtb, kernel):
        self.mode = mode
        self.dtb = dtb
        self.kernel = kernel
        self.done = False
        self.vtime = 0
        self.wall_start = 0
        self.wall_end = 0
        self.latencies = []
        self.overhead_latencies = []
        self.uart_buffer = ""

    def run(self):
        qemu_cmd = [
            os.path.join(WORKSPACE_DIR, "scripts", "run.sh"),
            "--dtb", self.dtb,
            "--kernel", self.kernel,
            "-nographic",
            "-serial", "stdio",
            "-monitor", "none"
        ]

        if "slaved-suspend" in self.mode:
            qemu_cmd += ["-device", "zenoh-clock,mode=suspend,node=0,router=tcp/127.0.0.1:7447"]
        elif "slaved-icount" in self.mode:
            qemu_cmd += ["-icount", "shift=0,align=off,sleep=off",
                         "-device", "zenoh-clock,mode=icount,node=0,router=tcp/127.0.0.1:7447"]
        
        proc = subprocess.Popen(qemu_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        
        def output_reader(p):
            for line in p.stdout:
                self.uart_buffer += line
                if "BENCH START" in line:
                    # print(f"  {self.mode}: BENCH START detected")
                    pass
                if "EXIT" in line:
                    print(f"  {self.mode}: EXIT detected")
                    self.done = True
        
        def stderr_reader(p):
            for line in p.stderr:
                if "STALL" in line or "FATAL" in line:
                    print(f"    QEMU ERR: {line.strip()}")
        
        out_thread = threading.Thread(target=output_reader, args=(proc,))
        out_thread.daemon = True
        out_thread.start()

        err_thread = threading.Thread(target=stderr_reader, args=(proc,))
        err_thread.daemon = True
        err_thread.start()

        self.wall_start = time.time()
        
        if self.mode == "standalone":
            deadline = time.time() + 30
            while not self.done and proc.poll() is None:
                if time.time() > deadline:
                    print(f"  ERROR: {self.mode} timed out (30s)")
                    break
                time.sleep(0.1)
        else:
            # Zenoh slaved mode
            config = zenoh.Config()
            config.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7447"]')
            config.insert_json5("scouting/multicast/enabled", "false")
            session = zenoh.open(config)
            
            topic = "sim/clock/advance/0"
            quantum = 50_000_000 # 50ms
            
            # Wait for queryable
            deadline = time.time() + 15
            found = False
            while time.time() < deadline:
                r = list(session.get(topic, payload=pack_req(0), timeout=0.5))
                if r:
                    found = True
                    break
                time.sleep(0.1)
            
            if not found:
                print(f"  ERROR: Queryable {topic} not found")
                self.done = True
            else:
                # 1. Gift 0ns quanta until we see "BENCH START"
                start_wait = time.time()
                while "BENCH START" not in self.uart_buffer and (time.time() - start_wait) < 30:
                     replies = list(session.get(topic, payload=pack_req(0), timeout=2.0))
                     if not replies: break
                
                # Measure pure overhead (0ns quanta)
                for i in range(10):
                    start = time.perf_counter()
                    replies = list(session.get(topic, payload=pack_req(0), timeout=5.0))
                    if not replies:
                        break
                    self.overhead_latencies.append(time.perf_counter() - start)

                q_idx = 0
                while not self.done and proc.poll() is None:
                    lat_start = time.perf_counter()
                    replies = list(session.get(topic, payload=pack_req(quantum), timeout=15.0))
                    lat_end = time.perf_counter()
                    
                    if not replies:
                        print(f"  ERROR: {self.mode} Zenoh quantum {q_idx} timeout")
                        break
                    
                    self.latencies.append(lat_end - lat_start)
                    reply = replies[0]
                    if reply.ok is not None:
                        resp = unpack_rep(reply.ok.payload.to_bytes())
                        self.vtime = resp.current_vtime_ns
                        if resp.error_code != 0:
                             print(f"  ERROR: {self.mode} QEMU reported error {resp.error_code}")
                             break
                    elif reply.err is not None:
                        print(f"  ERROR: {self.mode} Zenoh error reply: {reply.err}")
                        break
                    
                    q_idx += 1
            
            session.close()

        self.wall_end = time.time()
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except:
            proc.kill()
        
        duration = self.wall_end - self.wall_start
        return duration, self.vtime

def main():
    dtb = os.path.join(SCRIPT_DIR, "minimal.dtb")
    kernel = os.path.join(SCRIPT_DIR, "bench.elf")
    
    # Compile DTB
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", dtb, os.path.join(WORKSPACE_DIR, "test/phase1/minimal.dts")], check=True)

    # Start Router
    router_proc = subprocess.Popen(["python3", os.path.join(WORKSPACE_DIR, "tests", "zenoh_router_persistent.py")], 
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)

    try:
        results = {}
        overheads = {}
        
        # We focus on icount for benchmarking as suspend is not deterministic on wall-clock
        modes = ["standalone", "slaved-icount", "slaved-icount-2"]
        
        for mode in modes:
            print(f"--- Running Benchmark: {mode} ---")
            runner = BenchmarkRunner(mode, dtb, kernel)
            duration, vtime = runner.run()
            results[mode] = (duration, vtime)
            if runner.overhead_latencies:
                overheads[mode] = sum(runner.overhead_latencies) / len(runner.overhead_latencies)
            print(f"  Duration: {duration:.2f} s")
            if vtime:
                print(f"  VTime: {vtime/1e9:.6f} s")
            
        print("\n=== Performance Summary ===")
        total_instr = results["slaved-icount"][1]
        if total_instr == 0:
            print("ERROR: Could not get instruction count from icount mode!")
            sys.exit(1)

        print(f"Reference Instructions (icount 1): {total_instr:,}")
        
        # Determinism check
        if results["slaved-icount"][1] != results["slaved-icount-2"][1]:
            print(f"CRITICAL: Non-deterministic instruction count!")
            print(f"  Run 1: {results['slaved-icount'][1]}")
            print(f"  Run 2: {results['slaved-icount-2'][1]}")
        else:
            print("Determinism check: PASSED (Instruction counts match)")

        for mode, (duration, _) in results.items():
            if duration > 0:
                ips = total_instr / duration
                print(f"{mode:20}: {ips/1e6:7.2f} MIPS")
        
        print("\n=== Zenoh Overhead (0ns quantum) ===")
        for mode, avg_lat in overheads.items():
            print(f"{mode:20}: {avg_lat*1000:6.3f} ms")

    finally:
        router_proc.terminate()
        router_proc.wait()

if __name__ == "__main__":
    main()
