#!/usr/bin/env bash
# tests/fixtures/guest_apps/clock_suspend/tcp_router_test.sh — Explicit TCP router connectivity test.
#
# Verifies that the router= property on the clock device causes QEMU to
# connect via TCP rather than falling back to multicast peer discovery.
#
# This test is the key guard against multi-container deployment failures: in
# Docker Compose environments (especially macOS) multicast UDP is dropped
# between containers, so the router= TCP path is the ONLY reliable route.
#
# Approach:
#   1. Start a Zenoh listener on a dynamic endpoint with multicast disabled.
#   2. Start QEMU with router=dynamic_endpoint (also multicast disabled via
#      the router= code path in clock.c).
#   3. Run a full clock-advance handshake through the TCP connection.
#   4. Verify the reply contains a monotonically increasing vtime.
#
# A dynamically allocated port is used to avoid colliding with other processes
# during parallel test execution.

set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
tests/fixtures/guest_apps/clock_suspend/tcp_router_test.sh — Explicit TCP router connectivity test.

Verifies that the router= property on the clock device causes QEMU to
connect via TCP rather than falling back to multicast peer discovery.

This test is the key guard against multi-container deployment failures: in
Docker Compose environments (especially macOS) multicast UDP is dropped
between containers, so the router= TCP path is the ONLY reliable route.

Approach:
  1. Start a Zenoh listener on a dynamic endpoint with multicast disabled.
  2. Start QEMU with router=dynamic_endpoint (also multicast disabled via
     the router= code path in clock.c).
  3. Run a full clock-advance handshake through the TCP connection.
  4. Verify the reply contains a monotonically increasing vtime.

A dynamically allocated port is used to avoid colliding with other processes
during parallel test execution.
TEST_DOC_BLOCK
echo "=============================================================================="


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
TMPDIR_LOCAL="$(mktemp -d /tmp/clock_suspend_tcp_XXXXXX)"
QEMU_PID=""

cleanup() {
    [[ -n "$QEMU_PID" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

# ── Firmware ────────────────────────────────────────────────────────────────

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
    model = "virtmcu-tcp-test";
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

# Find a free IP and port
ROUTER_IP=$(python3 "$SCRIPTS_DIR/get-free-port.py" --ip)
ROUTER_PORT=$(python3 "$SCRIPTS_DIR/get-free-port.py" --port)
ROUTER_ENDPOINT="tcp/$ROUTER_IP:$ROUTER_PORT"
export ROUTER_IP ROUTER_PORT ROUTER_ENDPOINT

# ── TCP listener + clock-advance handshake ───────────────────────────────────
#
# The listener uses TCP only (multicast disabled).  QEMU must connect via the
# router= property; if it falls back to multicast the GET never arrives and
# the test times out.

cat > "$TMPDIR_LOCAL/tcp_clock_test.py" <<'PY_EOF'
"""
Listen on a dynamic endpoint.
Send one clock-advance GET to QEMU and verify the reply vtime > 0.
"""
import sys
import time
import os
import zenoh

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
TOOLS_DIR = os.path.join(os.path.dirname(os.path.dirname(SCRIPT_DIR)), "tools")
if TOOLS_DIR not in sys.path:
    sys.path.append(TOOLS_DIR)

from vproto import ClockAdvanceReq, ClockReadyResp

ROUTER_ENDPOINT = os.environ.get("ROUTER_ENDPOINT")
TOPIC       = "sim/clock/advance/0"
DELTA_NS    = 1_000_000   # 1 ms
TIMEOUT_S   = 15.0


def pack_req(delta_ns: int) -> bytes:
    req = ClockAdvanceReq(delta_ns=delta_ns, mujoco_time_ns=0, quantum_number=0)
    return req.pack()


def unpack_rep(data: bytes) -> int:
    resp = ClockReadyResp.unpack(data)
    return resp.current_vtime_ns


def main() -> None:
    config = zenoh.Config()
    config.insert_json5("listen/endpoints", f'["{ROUTER_ENDPOINT}"]')
    config.insert_json5("scouting/multicast/enabled", "false")

    session = zenoh.open(config)

    deadline = time.time() + TIMEOUT_S
    while time.time() < deadline:
        try:
            replies = list(session.get(TOPIC, payload=pack_req(DELTA_NS), timeout=2.0))
            if replies:
                reply = replies[0]
                if hasattr(reply, "ok") and reply.ok is not None:
                    vtime = unpack_rep(reply.ok.payload.to_bytes())
                    if vtime < DELTA_NS:
                        sys.stderr.write(f"FAIL: vtime {vtime} < expected {DELTA_NS}\n")
                        session.close()
                        sys.exit(1)
                    sys.stdout.write(f"PASS: TCP clock-advance vtime={vtime} ns\n")
                    session.close()
                    sys.exit(0)
        except Exception as exc:
            sys.stdout.write(f"Retry: {exc}\n")
        time.sleep(0.5)

    sys.stderr.write("TIMEOUT: QEMU did not connect via TCP router\n")
    session.close()
    sys.exit(1)


if __name__ == "__main__":
    main()
PY_EOF

# ── Start the TCP listener ────────────────────────────────────────────────────

echo "=== TCP router test: starting listener on ${ROUTER_ENDPOINT} ==="
python3 "$TMPDIR_LOCAL/tcp_clock_test.py" &
LISTENER_PID=$!

# Wait for the listener to bind (deterministic polling)
timeout 5 bash -c 'until ss -tln | grep -q ":${ROUTER_PORT} "; do sleep 0.1; done' || (echo "Router failed to bind to ${ROUTER_PORT}" && exit 1)

# ── Start QEMU ───────────────────────────────────────────────────────────────
#
# router=$ROUTER_ENDPOINT sets connect/endpoints in zenoh-c config and
# disables multicast scouting — both sides must agree on TCP-only mode.

"$SCRIPTS_DIR/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -device "virtmcu-clock,mode=slaved-suspend,router=$ROUTER_ENDPOINT,node=0" \
    -nographic \
    -monitor none \
    > "$TMPDIR_LOCAL/qemu_tcp.log" 2>&1 &
QEMU_PID=$!

if wait "$LISTENER_PID"; then
    echo "=== TCP router test PASSED ==="
else
    echo "=== TCP router test FAILED ==="
    cat "$TMPDIR_LOCAL/qemu_tcp.log"
    exit 1
fi
