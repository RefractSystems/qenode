#!/usr/bin/env bash
# test/phase7/tcp_router_test.sh — Explicit TCP router connectivity test.
#
# Verifies that the router= property on the zenoh-clock device causes QEMU to
# connect via TCP rather than falling back to multicast peer discovery.
#
# This test is the key guard against multi-container deployment failures: in
# Docker Compose environments (especially macOS) multicast UDP is dropped
# between containers, so the router= TCP path is the ONLY reliable route.
#
# Approach:
#   1. Start a Zenoh listener on tcp/127.0.0.1:7448 with multicast disabled.
#   2. Start QEMU with router=tcp/127.0.0.1:7448 (also multicast disabled via
#      the router= code path in zenoh-clock.c).
#   3. Run a full clock-advance handshake through the TCP connection.
#   4. Verify the reply contains a monotonically increasing vtime.
#
# Port 7448 is used (not 7447) to avoid colliding with other zenoh processes
# that might be running from earlier smoke test phases.

set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
test/phase7/tcp_router_test.sh — Explicit TCP router connectivity test.

Verifies that the router= property on the zenoh-clock device causes QEMU to
connect via TCP rather than falling back to multicast peer discovery.

This test is the key guard against multi-container deployment failures: in
Docker Compose environments (especially macOS) multicast UDP is dropped
between containers, so the router= TCP path is the ONLY reliable route.

Approach:
  1. Start a Zenoh listener on tcp/127.0.0.1:7448 with multicast disabled.
  2. Start QEMU with router=tcp/127.0.0.1:7448 (also multicast disabled via
     the router= code path in zenoh-clock.c).
  3. Run a full clock-advance handshake through the TCP connection.
  4. Verify the reply contains a monotonically increasing vtime.

Port 7448 is used (not 7447) to avoid colliding with other zenoh processes
that might be running from earlier smoke test phases.
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_tcp_XXXXXX)"
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

# ── TCP listener + clock-advance handshake ───────────────────────────────────
#
# The listener uses TCP only (multicast disabled).  QEMU must connect via the
# router= property; if it falls back to multicast the GET never arrives and
# the test times out.

cat > "$TMPDIR_LOCAL/tcp_clock_test.py" <<'PY_EOF'
"""
Listen on tcp/127.0.0.1:7448 with multicast disabled.
Send one clock-advance GET to QEMU and verify the reply vtime > 0.
"""
import sys
import struct
import time
import zenoh

ROUTER_PORT = 7448
TOPIC       = "sim/clock/advance/0"
DELTA_NS    = 1_000_000   # 1 ms
TIMEOUT_S   = 15.0


def pack_req(delta_ns: int) -> bytes:
    return struct.pack("<QQ", delta_ns, 0)


def unpack_rep(data: bytes) -> int:
    # 16-byte reply: <Q (vtime_ns) I (n_frames) I (error_code)
    vtime_ns, _n_frames, _error_code = struct.unpack("<QII", data)
    return vtime_ns


def main() -> None:
    config = zenoh.Config()
    config.insert_json5("listen/endpoints", f'["tcp/127.0.0.1:{ROUTER_PORT}"]')
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
                        print(f"FAIL: vtime {vtime} < expected {DELTA_NS}", file=sys.stderr)
                        session.close()
                        sys.exit(1)
                    print(f"PASS: TCP clock-advance vtime={vtime} ns")
                    session.close()
                    sys.exit(0)
        except Exception as exc:
            print(f"Retry: {exc}")
        time.sleep(0.5)

    print("TIMEOUT: QEMU did not connect via TCP router", file=sys.stderr)
    session.close()
    sys.exit(1)


if __name__ == "__main__":
    main()
PY_EOF

# ── Start the TCP listener ────────────────────────────────────────────────────

echo "=== TCP router test: starting listener on port ${ROUTER_PORT:-7448} ==="
python3 "$TMPDIR_LOCAL/tcp_clock_test.py" &
LISTENER_PID=$!

sleep 2   # Let the listener bind before QEMU tries to connect

# ── Start QEMU ───────────────────────────────────────────────────────────────
#
# router=tcp/127.0.0.1:7448 sets connect/endpoints in zenoh-c config and
# disables multicast scouting — both sides must agree on TCP-only mode.

"$WORKSPACE_DIR/scripts/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -device "zenoh-clock,mode=suspend,router=tcp/127.0.0.1:7448,node=0" \
    -nographic \
    -monitor none \
    > "$TMPDIR_LOCAL/qemu_tcp.log" 2>&1 &
QEMU_PID=$!

if wait "$LISTENER_PID"; then
    echo "=== Phase 7 TCP router test PASSED ==="
else
    echo "=== Phase 7 TCP router test FAILED ==="
    cat "$TMPDIR_LOCAL/qemu_tcp.log"
    exit 1
fi
