"""
SOTA Test Module: verify_control

Context:
This module implements tests for the verify_control subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of verify_control.
"""

# tests/fixtures/guest_apps/actuator/verify_control.py
import logging
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import zenoh

logger = logging.getLogger(__name__)


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent.parent


def main():
    logger.info("[Test] Starting Zenoh control verification...")

    workspace_dir = _find_workspace_root(Path(__file__).resolve())
    router_script = workspace_dir / "tests" / "zenoh_router_persistent.py"

    # 1. Start Zenoh router
    logger.info("[Test] Starting Zenoh router...")
    router_proc = subprocess.Popen([sys.executable, router_script, "ZENOH_ROUTER_ENDPOINT"])
    time.sleep(2)

    # 2. Open Zenoh session
    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", '["ZENOH_ROUTER_ENDPOINT"]')
    conf.insert_json5("scouting/multicast/enabled", "false")
    session = zenoh.open(conf)

    received_msgs = []

    def on_sample(sample):
        topic = str(sample.key_expr)
        payload = sample.payload.to_bytes()
        if len(payload) < 8:
            return
        vtime_ns = int.from_bytes(payload[:8], "little")
        data_bytes = payload[8:]
        import array

        a = array.array("d", data_bytes)
        vals = a.tolist()

        logger.info(f"[Zenoh] Received: topic={topic}, vtime={vtime_ns}, vals={vals}")
        received_msgs.append({"topic": topic, "vtime": vtime_ns, "vals": vals})

    # Subscribe to firmware/control/0/**
    session.declare_subscriber("firmware/control/0/**", on_sample)

    # 2. Run QEMU
    script_dir = Path(os.path.realpath(__file__)).parent
    workspace_dir = Path(Path(script_dir).parent.parent.parent.parent)
    run_sh_path = os.environ.get("RUN_SH") or str(workspace_dir / "scripts" / "run.sh")

    dtb = Path(script_dir) / "board.dtb"
    kernel = Path(script_dir) / "actuator.elf"

    cmd = [
        run_sh_path,
        "--dtb",
        dtb,
        "--kernel",
        kernel,
        "-nographic",
        "-monitor",
        "none",
        "-serial",
        "stdio",
        # Use clock in standalone mode (no TimeAuthority) so QEMU runs at full speed
        # Actually, if we don't provide -device virtmcu-clock, it runs standalone.
    ]

    logger.info(f"[Test] Running: {' '.join(map(str, cmd))}")
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, preexec_fn=os.setsid)

    # 3. Wait for output and messages
    start_time = time.time()
    timeout = 10  # seconds

    success_1 = False
    success_2 = False

    try:
        while time.time() - start_time < timeout:
            if proc.poll() is not None:
                logger.info(f"[QEMU] Process exited unexpectedly with code {proc.returncode}")
                break

            # Non-blocking read would be better, but for this simple test, we just rely on timeout
            line = proc.stdout.readline()
            if line:
                logger.info(f"[QEMU] {line.strip()}")
                if "Control signal 2 sent." in line:
                    # Give it a bit of time for Zenoh to deliver
                    time.sleep(1)
                    break

            if len(received_msgs) >= 2:
                break

            time.sleep(0.1)
    except Exception as e:
        logger.info(f"[Test] Exception: {e}")
    finally:
        # Kill QEMU
        if proc.poll() is None:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                proc.wait(timeout=5)
            except Exception:
                pass

        # Kill router
        if router_proc.poll() is None:
            router_proc.terminate()
            router_proc.wait(timeout=5)

    # 4. Verify results
    for msg in received_msgs:
        if msg["topic"] == "firmware/control/0/42" and abs(msg["vals"][0] - 3.14) < 0.001:
            success_1 = True
        elif msg["topic"] == "firmware/control/0/99" and len(msg["vals"]) == 3 and msg["vals"] == [1.0, 2.0, 3.0]:
            success_2 = True

    if success_1 and success_2:
        logger.info("[Test] SUCCESS: All control signals verified.")
        sys.exit(0)
    else:
        logger.info(f"[Test] FAILURE: success_1={success_1}, success_2={success_2}")
        logger.info(f"[Test] Received {len(received_msgs)} messages total.")
        sys.exit(1)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
