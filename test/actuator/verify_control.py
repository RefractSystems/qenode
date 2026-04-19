# test/actuator/verify_control.py
import os
import signal
import struct
import subprocess
import sys
import time

import zenoh


def main():
    print("[Test] Starting Zenoh control verification...")

    workspace_dir = os.path.dirname(os.path.dirname(os.path.dirname(os.path.realpath(__file__))))
    router_script = os.path.join(workspace_dir, "tests", "zenoh_router_persistent.py")

    # 1. Start Zenoh router
    print("[Test] Starting Zenoh router...")
    router_proc = subprocess.Popen([sys.executable, router_script, "tcp/127.0.0.1:7450"])
    time.sleep(2)

    # 2. Open Zenoh session
    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7450"]')
    conf.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(conf)

    received_msgs = []

    def on_sample(sample):
        topic = str(sample.key_expr)
        payload = sample.payload.to_bytes()
        if len(payload) < 8:
            return
        vtime_ns = struct.unpack("<Q", payload[:8])[0]
        data_bytes = payload[8:]
        n_doubles = len(data_bytes) // 8
        vals = struct.unpack("<" + "d" * n_doubles, data_bytes)

        print(f"[Zenoh] Received: topic={topic}, vtime={vtime_ns}, vals={vals}")
        received_msgs.append({"topic": topic, "vtime": vtime_ns, "vals": vals})

    # Subscribe to firmware/control/0/**
    session.declare_subscriber("firmware/control/0/**", on_sample)

    # 2. Run QEMU
    script_dir = os.path.dirname(os.path.realpath(__file__))
    workspace_dir = os.path.dirname(os.path.dirname(script_dir))
    run_sh = os.path.join(workspace_dir, "scripts", "run.sh")

    dtb = os.path.join(script_dir, "board.dtb")
    kernel = os.path.join(script_dir, "actuator.elf")

    cmd = [
        run_sh,
        "--dtb",
        dtb,
        "--kernel",
        kernel,
        "-nographic",
        "-monitor",
        "none",
        "-serial",
        "stdio",
        # Use zenoh-clock in standalone mode (no TimeAuthority) so QEMU runs at full speed
        # Actually, if we don't provide -device zenoh-clock, it runs standalone.
    ]

    print(f"[Test] Running: {' '.join(cmd)}")
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, preexec_fn=os.setsid)

    # 3. Wait for output and messages
    start_time = time.time()
    timeout = 10  # seconds

    success_1 = False
    success_2 = False

    try:
        while time.time() - start_time < timeout:
            line = proc.stdout.readline()
            if line:
                print(f"[QEMU] {line.strip()}")
                if "Control signal 2 sent." in line:
                    # Give it a bit of time for Zenoh to deliver
                    time.sleep(1)
                    break

            if len(received_msgs) >= 2:
                break

            time.sleep(0.1)
    except Exception as e:
        print(f"[Test] Exception: {e}")
    finally:
        # Kill QEMU
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        proc.wait()

        # Kill router
        router_proc.terminate()
        router_proc.wait()

    # 4. Verify results
    for msg in received_msgs:
        if msg["topic"] == "firmware/control/0/42":
            if abs(msg["vals"][0] - 3.14) < 0.001:
                success_1 = True
        elif msg["topic"] == "firmware/control/0/99":
            if len(msg["vals"]) == 3 and msg["vals"] == (1.0, 2.0, 3.0):
                success_2 = True

    if success_1 and success_2:
        print("[Test] SUCCESS: All control signals verified.")
        sys.exit(0)
    else:
        print(f"[Test] FAILURE: success_1={success_1}, success_2={success_2}")
        print(f"[Test] Received {len(received_msgs)} messages total.")
        sys.exit(1)


if __name__ == "__main__":
    main()
