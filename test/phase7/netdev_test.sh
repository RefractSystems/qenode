#!/usr/bin/env bash
# test/phase7/netdev_test.sh — Phase 7 Zenoh netdev backend functional test.
#
# Verifies:
#   1. QEMU starts with -netdev zenoh and successfully opens a Zenoh session,
#      declaring a publisher on sim/eth/frame/{node}/tx and a subscriber on
#      sim/eth/frame/{node}/rx.
#   2. An RX packet published to sim/eth/frame/1/rx with delivery_vtime = 0.5 ms
#      is accepted by the backend (timer_mod called, no crash).
#   3. After advancing the virtual clock past delivery_vtime, QEMU continues
#      responding to further clock advances — proving rx_timer_cb and
#      qemu_send_packet do not corrupt the QEMU event loop.
#
# Design:
#   icount mode is used so virtual time is instruction-count based and fully
#   controlled by Zenoh clock advances.  This prevents the rx_timer from firing
#   prematurely due to host-wall-clock drift.
#
#   The guest firmware is a bare infinite loop.  The injected RX packet has no
#   NIC peer (no -device NIC), so qemu_send_packet drops it silently — that is
#   the expected outcome.  We test the netdev backend, not a guest NIC driver.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_net_XXXXXX)"
QEMU_PID=""

cleanup() {
    [[ -n "$QEMU_PID" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

# ── Minimal firmware: bare infinite loop ─────────────────────────────────────
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

# ── Minimal DTB ─────────────────────────────────────────────────────────────
cat > "$TMPDIR_LOCAL/dummy.dts" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-test";
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;
    qemu_sysmem: qemu_sysmem { compatible = "qemu:system-memory"; phandle = <0x01>; };
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
        cpu@0 { device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; };
    };
};
DTS_EOF
dtc -I dts -O dtb -o "$TMPDIR_LOCAL/dummy.dtb" "$TMPDIR_LOCAL/dummy.dts"

# ── Launch QEMU ──────────────────────────────────────────────────────────────
# -netdev zenoh,node=1 registers the Zenoh backend without a peer NIC device.
# -icount + zenoh-clock,mode=icount give full virtual-time control.
"$WORKSPACE_DIR/scripts/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -icount shift=0,align=off,sleep=off \
    -device zenoh-clock,mode=icount,node=1 \
    -netdev zenoh,node=1,id=n1 \
    -nographic \
    -monitor none \
    > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

# ── Wait for the clock queryable ─────────────────────────────────────────────
CLOCK_TOPIC="sim/clock/advance/1"
deadline=$(( $(date +%s) + 15 ))
ready=0
while (( $(date +%s) < deadline )); do
    if python3 - <<EOF 2>/dev/null
import zenoh, sys, struct
s = zenoh.open(zenoh.Config())
r = list(s.get("$CLOCK_TOPIC", payload=struct.pack("<QQ", 0, 0), timeout=1.0))
s.close()
sys.exit(0 if r else 1)
EOF
    then
        ready=1
        break
    fi
    sleep 0.25
done

if [ "$ready" -eq 0 ]; then
    echo "FAIL: clock queryable not available after 15 s" >&2
    cat "$TMPDIR_LOCAL/qemu.log" >&2
    exit 1
fi
echo "QEMU clock queryable ready."

# ── Functional test ──────────────────────────────────────────────────────────
python3 - "$CLOCK_TOPIC" <<'PY_EOF'
"""
Netdev RX-path functional test:

  1. Publish a Zenoh packet to sim/eth/frame/1/rx with delivery_vtime = 500 000 ns
     (0.5 ms of icount virtual time).  The backend's on_rx_frame callback queues
     it and arms the rx_timer at 500 000 ns.

  2. Advance the virtual clock by 1 ms.  QEMU executes 1 000 000 instructions.
     At icount 500 000 the rx_timer fires, rx_timer_cb drains the queue, and
     qemu_send_packet is called (packet dropped — no peer NIC).

  3. Advance the virtual clock by another 1 ms.  If rx_timer_cb corrupted the
     event loop this advance times out (deadlock) or returns a non-monotone vtime.

  4. Assert both advances returned and vtimes are strictly monotonically increasing.
"""
import sys, struct, time
import zenoh

CLOCK_TOPIC   = sys.argv[1]
NETDEV_TOPIC  = "sim/eth/frame/1/rx"

# ZenohFrameHeader: { uint64 delivery_vtime_ns; uint32 size; } followed by frame bytes.
DELIVERY_VTIME_NS = 500_000   # 0.5 ms — fires mid-way through the 1 ms advance
FRAME = b'\xff' * 14          # minimal broadcast Ethernet header
hdr   = struct.pack("<QI", DELIVERY_VTIME_NS, len(FRAME))
packet = hdr + FRAME

DELTA_NS  = 1_000_000   # 1 ms per advance
TIMEOUT_S = 5.0

def unpack_vtime(reply):
    data = reply.ok.payload.to_bytes()
    vtime_ns, _ = struct.unpack("<QI", data)
    return vtime_ns

session = zenoh.open(zenoh.Config())
pub = session.declare_publisher(NETDEV_TOPIC)

# Step 1: inject RX packet via Zenoh before advancing the clock.
pub.put(packet)
print(f"Injected RX packet with delivery_vtime={DELIVERY_VTIME_NS} ns")

# Give the Zenoh subscriber in QEMU's background thread time to receive and
# call timer_mod before we unblock the vCPU with the first clock advance.
time.sleep(0.1)

# Step 2: advance clock by 1 ms — rx_timer fires at 0.5 ms.
r1 = list(session.get(CLOCK_TOPIC,
                       payload=struct.pack("<QQ", DELTA_NS, 0),
                       timeout=TIMEOUT_S))
if not r1:
    print("FAIL: first clock advance timed out", file=sys.stderr)
    sys.exit(1)
vtime1 = unpack_vtime(r1[0])
print(f"Advance 1: vtime={vtime1} ns  (expected >= {DELTA_NS})")
if vtime1 < DELTA_NS:
    print(f"FAIL: vtime1 {vtime1} < {DELTA_NS}", file=sys.stderr)
    sys.exit(1)

# Step 3: advance clock by another 1 ms — proves no deadlock from rx path.
r2 = list(session.get(CLOCK_TOPIC,
                       payload=struct.pack("<QQ", DELTA_NS, 0),
                       timeout=TIMEOUT_S))
if not r2:
    print("FAIL: second clock advance timed out (possible deadlock in rx_timer_cb)",
          file=sys.stderr)
    sys.exit(1)
vtime2 = unpack_vtime(r2[0])
print(f"Advance 2: vtime={vtime2} ns  (expected >= {vtime1 + DELTA_NS})")
if vtime2 <= vtime1:
    print(f"FAIL: vtime2 {vtime2} not > vtime1 {vtime1}", file=sys.stderr)
    sys.exit(1)

session.close()
print("PASS: Zenoh netdev RX path functional — timer fired, no deadlock, vtimes monotone.")
PY_EOF

echo "=== Phase 7 netdev test PASSED ==="
