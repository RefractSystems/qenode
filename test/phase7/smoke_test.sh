#!/usr/bin/env bash
# test/phase7/smoke_test.sh — Phase 7 smoke test: zenoh-clock suspend & icount modes.
#
# Verifies:
#   1. QEMU starts and registers the sim/clock/advance/0 Zenoh queryable.
#   2. Two successive clock-advance queries return monotonically increasing vtimes.
#   3. Each returned vtime is >= the cumulative delta supplied (basic sanity).
#   4. Both suspend mode and icount mode pass the above checks.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_XXXXXX)"
QEMU_PID=""

cleanup() {
    [[ -n "$QEMU_PID" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

# ── Firmware ────────────────────────────────────────────────────────────────

# Minimal linker script — no cross-phase dependency.
cat > "$TMPDIR_LOCAL/linker.ld" <<'LD_EOF'
SECTIONS {
    . = 0x40000000;
    .text : { *(.text) }
}
LD_EOF

cat > "$TMPDIR_LOCAL/firmware.S" <<'ASM_EOF'
.global _start
_start:
loop:
    b loop
ASM_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib \
    -T "$TMPDIR_LOCAL/linker.ld" \
    "$TMPDIR_LOCAL/firmware.S" \
    -o "$TMPDIR_LOCAL/firmware.elf"

# ── Device tree ─────────────────────────────────────────────────────────────

cat > "$TMPDIR_LOCAL/dummy.dts" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-test";
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;
    qemu_sysmem: qemu_sysmem {
        compatible = "qemu:system-memory";
        phandle = <0x01>;
    };
    chosen {};
    memory@40000000 {
        compatible = "qemu-memory-region";
        qemu,ram = <0x01>;
        container = <0x01>;
        reg = <0x0 0x40000000 0x0 0x10000000>;
    };
    cpus {
        #address-cells = <1>;
        #size-cells = <0>;
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
            memory = <0x01>;
        };
    };
};
DTS_EOF

dtc -I dts -O dtb -o "$TMPDIR_LOCAL/dummy.dtb" "$TMPDIR_LOCAL/dummy.dts"

# ── Python test script ───────────────────────────────────────────────────────

cat > "$TMPDIR_LOCAL/test_clock.py" <<'PY_EOF'
"""
Send two successive clock-advance queries and verify:
  - Both queries succeed (no error reply).
  - vtime after Q1 >= delta1.
  - vtime after Q2 >= delta1 + delta2 (monotone).
"""
import zenoh
import struct
import sys

DELTA1_NS = 1_000_000   # 1 ms
DELTA2_NS = 2_000_000   # 2 ms
TOPIC     = "sim/clock/advance/0"
TIMEOUT_S = 5.0         # generous timeout for CI

def pack_req(delta_ns):
    return struct.pack("<QQ", delta_ns, 0)

def unpack_rep(data):
    vtime_ns, n_frames = struct.unpack("<QI", data)
    return vtime_ns

def send_query(session, delta_ns, label):
    replies = list(session.get(TOPIC, payload=pack_req(delta_ns), timeout=TIMEOUT_S))
    if not replies:
        print(f"{label}: TIMEOUT — no reply received", file=sys.stderr)
        sys.exit(1)
    reply = replies[0]
    if not hasattr(reply, "ok"):
        print(f"{label}: ERROR reply: {reply}", file=sys.stderr)
        sys.exit(1)
    return unpack_rep(reply.ok.payload.to_bytes())

def main():
    session = zenoh.open(zenoh.Config())

    vtime1 = send_query(session, DELTA1_NS, "Q1")
    print(f"Q1 vtime = {vtime1} ns  (expected >= {DELTA1_NS})")
    if vtime1 < DELTA1_NS:
        print(f"FAIL: Q1 vtime {vtime1} < expected {DELTA1_NS}", file=sys.stderr)
        sys.exit(1)

    vtime2 = send_query(session, DELTA2_NS, "Q2")
    print(f"Q2 vtime = {vtime2} ns  (expected >= {DELTA1_NS + DELTA2_NS})")
    if vtime2 <= vtime1:
        print(f"FAIL: Q2 vtime {vtime2} not > Q1 vtime {vtime1}", file=sys.stderr)
        sys.exit(1)
    if vtime2 < DELTA1_NS + DELTA2_NS:
        print(f"FAIL: Q2 vtime {vtime2} < cumulative {DELTA1_NS + DELTA2_NS}", file=sys.stderr)
        sys.exit(1)

    session.close()
    print("PASS")

if __name__ == "__main__":
    main()
PY_EOF

# ── Helper: wait until the Zenoh queryable is reachable ──────────────────────

wait_for_queryable() {
    local topic="$1"
    local deadline=$(( $(date +%s) + 15 ))
    while (( $(date +%s) < deadline )); do
        if python3 - <<EOF 2>/dev/null
import zenoh, sys, struct
s = zenoh.open(zenoh.Config())
p = struct.pack("<QQ", 0, 0)
r = list(s.get("$topic", payload=p, timeout=1.0))
s.close()
sys.exit(0 if r else 1)
EOF
        then
            return 0
        fi
        sleep 0.25
    done
    echo "ERROR: queryable '$topic' not available after 15 s" >&2
    cat "$TMPDIR_LOCAL/qemu_suspend.log" 2>/dev/null || true
    cat "$TMPDIR_LOCAL/qemu_icount.log" 2>/dev/null || true
    return 1
}

# ── Run: suspend mode ────────────────────────────────────────────────────────

echo "=== suspend mode ==="
"$WORKSPACE_DIR/scripts/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -device zenoh-clock,mode=suspend,node=0 \
    -nographic \
    -monitor none \
    > "$TMPDIR_LOCAL/qemu_suspend.log" 2>&1 &
QEMU_PID=$!

wait_for_queryable "sim/clock/advance/0"
python3 "$TMPDIR_LOCAL/test_clock.py"

kill -9 "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

# ── Run: icount mode ─────────────────────────────────────────────────────────

echo "=== icount mode ==="
"$WORKSPACE_DIR/scripts/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -icount shift=0,align=off,sleep=off \
    -device zenoh-clock,mode=icount,node=0 \
    -nographic \
    -monitor none \
    > "$TMPDIR_LOCAL/qemu_icount.log" 2>&1 &
QEMU_PID=$!

wait_for_queryable "sim/clock/advance/0"
python3 "$TMPDIR_LOCAL/test_clock.py"

kill -9 "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

echo "=== Phase 7 smoke test (Zenoh Clock Suspend/icount) PASSED ==="

echo "=== Running Phase 7 determinism test ==="
"$SCRIPT_DIR/determinism_test.sh"

echo "=== Running Phase 7 netdev test ==="
"$SCRIPT_DIR/netdev_test.sh"

echo "=== Running Phase 7 TCP router test ==="
"$SCRIPT_DIR/tcp_router_test.sh"

echo "=== All Phase 7 tests PASSED ==="
