#!/usr/bin/env bash
# tutorial/lesson16-performance/src/jitter_test.sh — Jitter Injection Determinism Test
#
# Proves that virtmcu's virtual-time gating correctly neutralizes host Zenoh
# jitter.  The test:
#   1. Starts a real Zenoh router (used by QEMU and the TimeAuthority mock).
#   2. Starts jitter_proxy.py on a separate port, which adds ±200 µs random
#      delays to every clock-advance reply.
#   3. Runs NUM_RUNS independent slaved-icount benchmark runs through the proxy.
#   4. Verifies all exit_vtime_ns values are identical (byte-perfect determinism).
#
# A failure here indicates that the virtual-time gating logic allowed host
# jitter to affect the guest instruction count — a correctness regression.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Find workspace root (robustly)
_search_dir="$SCRIPT_DIR"
while [[ "$_search_dir" != "/" ]]; do
    if [[ -f "$_search_dir/scripts/common.sh" ]]; then
        source "$_search_dir/scripts/common.sh"
        break
    fi
    _search_dir=$(dirname "$_search_dir")
done

if [[ -z "${WORKSPACE_DIR:-}" ]]; then
    echo "ERROR: Could not find scripts/common.sh" >&2
    exit 1
fi

# Number of independent runs; each must produce identical exit_vtime_ns.
NUM_RUNS=5

# Jitter proxy configuration.
MAX_JITTER_US=200

ROUTER_PID=0
PROXY_PID=0

cleanup() {
    [[ $ROUTER_PID -ne 0 ]] && kill "$ROUTER_PID" 2>/dev/null || true
    [[ $PROXY_PID  -ne 0 ]] && kill "$PROXY_PID"  2>/dev/null || true
    bash "$SCRIPTS_DIR/cleanup-sim.sh" --quiet 2>/dev/null || true
}
trap cleanup EXIT

# ─── helpers ─────────────────────────────────────────────────────────────────

log_pass() { echo "PASS: $1"; }
log_fail() { echo "FAIL: $1" >&2; exit 1; }

free_endpoint() {
    python3 "$SCRIPTS_DIR/get-free-port.py" --endpoint --proto "tcp/"
}

# ─── build bench firmware ────────────────────────────────────────────────────

echo "--- Building bench firmware ---"
make -C "$SCRIPT_DIR/.." bench.elf

# ─── start Zenoh router ──────────────────────────────────────────────────────

ROUTER_URL=$(free_endpoint)
echo "--- Starting Zenoh router on ${ROUTER_URL} ---"
python3 "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "$ROUTER_URL" &
ROUTER_PID=$!
sleep 2

# ─── start jitter proxy ──────────────────────────────────────────────────────

PROXY_URL=$(free_endpoint)
echo "--- Starting jitter proxy on ${PROXY_URL} (upstream: ${ROUTER_URL}) ---"
python3 "$SCRIPT_DIR/jitter_proxy.py" "$ROUTER_URL" "$PROXY_URL" "$MAX_JITTER_US" &
PROXY_PID=$!
sleep 2

# ─── run benchmark through proxy ─────────────────────────────────────────────

echo "--- Running ${NUM_RUNS} benchmark runs through jitter proxy ---"

VTIME_VALUES=()
for run in $(seq 1 $NUM_RUNS); do
    echo "  Run ${run}/${NUM_RUNS}..."

    # Run bench.py with PROXY as the router so QEMU routes through the jitter layer.
    # The TimeAuthority in bench.py connects to ROUTER directly; the proxy intercepts
    # QEMU's queries and adds jitter before forwarding to TimeAuthority.
    vtime=$(
        python3 - <<PYEOF 2>/dev/null
import os, sys, subprocess, threading, time
import zenoh

sys.path.insert(0, os.path.join("$WORKSPACE_DIR", "tools"))
from vproto import ClockAdvanceReq, ClockReadyResp

SCRIPT_DIR  = "$SCRIPT_DIR"
WORKSPACE   = "$WORKSPACE_DIR"
ROUTER_URL  = "$ROUTER_URL"   # TimeAuthority connects here
PROXY_URL   = "$PROXY_URL"    # QEMU connects here

QUANTUM_NS  = 10_000_000
MAX_QUANTA  = 2000

dtb = os.path.join(os.path.dirname(SCRIPT_DIR), "minimal.dtb")
kernel = os.path.join(os.path.dirname(SCRIPT_DIR), "bench.elf")

exit_event = threading.Event()
exit_vtime = [0]

def reader(proc):
    for line in proc.stdout:
        if "EXIT" in line:
            exit_event.set()

RUN_SH = os.environ.get("RUN_SH") or os.path.join(WORKSPACE, "scripts", "run.sh")
cmd = [RUN_SH, "--dtb", dtb, "--kernel", kernel,
       "-nographic", "-serial", "stdio", "-monitor", "none",
       "-icount", "shift=0,align=off,sleep=off",
       "-device", f"virtmcu-clock,mode=slaved-suspend,node=0,router={PROXY_URL}"]

proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
threading.Thread(target=reader, args=(proc,), daemon=True).start()

# TimeAuthority: connect directly to upstream router (bypassing proxy).
cfg = zenoh.Config()
cfg.insert_json5("connect/endpoints", f'["{ROUTER_URL}"]')
cfg.insert_json5("scouting/multicast/enabled", "false")
session = zenoh.open(cfg)

topic = "sim/clock/advance/0"
q_num = 0
payload0 = ClockAdvanceReq(delta_ns=0, mujoco_time_ns=0, quantum_number=q_num).pack()

# Wait for QEMU to reach first quantum boundary.
ready = False
deadline = time.perf_counter() + 30
while time.perf_counter() < deadline:
    replies = list(session.get(topic, payload=payload0, timeout=5.0))
    if replies and hasattr(replies[0], "ok") and replies[0].ok is not None:
        ready = True
        break
    time.sleep(0.2)
    q_num += 1
    payload0 = ClockAdvanceReq(delta_ns=0, mujoco_time_ns=0, quantum_number=q_num).pack()

if not ready:
    proc.terminate(); proc.wait()
    session.close()
    import sys
    sys.stdout.write("0\n")
    sys.exit(0)

current_q = q_num
for _ in range(MAX_QUANTA):
    if proc.poll() is not None:
        break
    payload_adv = ClockAdvanceReq(delta_ns=QUANTUM_NS, mujoco_time_ns=0, quantum_number=current_q).pack()
    replies = list(session.get(topic, payload=payload_adv, timeout=30.0))
    current_q += 1
    if not replies or not hasattr(replies[0], "ok") or replies[0].ok is None:
        break
    resp = ClockReadyResp.unpack(replies[0].ok.payload.to_bytes())
    if resp.error_code != 0:
        break
    if exit_event.is_set():
        exit_vtime[0] = resp.current_vtime_ns
        break

proc.terminate()
try:
    proc.wait(timeout=5)
except subprocess.TimeoutExpired:
    proc.kill()
session.close()
import sys
sys.stdout.write(str(exit_vtime[0]) + "\n")
PYEOF
    )

    echo "    exit_vtime_ns = ${vtime}"
    if [[ -z "$vtime" || "$vtime" == "0" ]]; then
        log_fail "Run ${run} produced no exit_vtime_ns — QEMU may have crashed"
    fi
    VTIME_VALUES+=("$vtime")
done

# ─── verify determinism ───────────────────────────────────────────────────────

echo "--- Verifying determinism across ${NUM_RUNS} runs ---"
REF="${VTIME_VALUES[0]}"
ALL_MATCH=true

for i in "${!VTIME_VALUES[@]}"; do
    v="${VTIME_VALUES[$i]}"
    if [[ "$v" != "$REF" ]]; then
        echo "  Run $((i+1)): exit_vtime_ns=${v}  ← MISMATCH (ref=${REF})"
        ALL_MATCH=false
    else
        echo "  Run $((i+1)): exit_vtime_ns=${v}  OK"
    fi
done

if $ALL_MATCH; then
    log_pass "All ${NUM_RUNS} runs produced identical exit_vtime_ns=${REF} despite ±${MAX_JITTER_US} µs jitter"
else
    log_fail "Determinism check failed — jitter neutralization is broken"
fi

echo "=== Jitter Test PASSED ==="
