#!/usr/bin/env bash
# tests/fixtures/guest_apps/clock_suspend/netdev_test.sh — Zenoh netdev backend functional test.
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
tests/fixtures/guest_apps/clock_suspend/netdev_test.sh — Zenoh netdev backend functional test.
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
TMPDIR_LOCAL="$(mktemp -d /tmp/clock_suspend_net_XXXXXX)"
QEMU_PID=""
ROUTER_PID=""

cleanup() {
    [[ -n "${QEMU_PID:-}" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    [[ -n "${ROUTER_PID:-}" ]] && kill -9 "$ROUTER_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}


# Minimal firmware
cat > "$TMPDIR_LOCAL/linker.ld" <<'LD_EOF'
SECTIONS { . = 0x40000000; .text : { *(.text) } }
LD_EOF
cat > "$TMPDIR_LOCAL/firmware.S" <<'ASM_EOF'
.global _start
_start: loop: b loop
ASM_EOF
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$TMPDIR_LOCAL/linker.ld" "$TMPDIR_LOCAL/firmware.S" -o "$TMPDIR_LOCAL/firmware.elf"

# Minimal DTB
cat > "$TMPDIR_LOCAL/dummy.dts" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-test"; compatible = "arm,generic-fdt"; #address-cells = <2>; #size-cells = <2>;
    qemu_sysmem: qemu_sysmem { compatible = "qemu:system-memory"; phandle = <0x01>; };
    chosen {};
    memory@40000000 { compatible = "qemu-memory-region"; qemu,ram = <0x01>; container = <0x01>; reg = <0x0 0x40000000 0x0 0x10000000>; };
    cpus { #address-cells = <1>; #size-cells = <0>; cpu@0 { device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; }; };
};
DTS_EOF
dtc -I dts -O dtb -o "$TMPDIR_LOCAL/dummy.dtb" "$TMPDIR_LOCAL/dummy.dts"

ENDPOINT=${1:-}
if [ -z "$ENDPOINT" ]; then
    ENDPOINT=$(python3 "$SCRIPTS_DIR/get-free-port.py" --endpoint --proto "tcp/")
fi

# Launch Router & QEMU
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "$ENDPOINT" &
ROUTER_PID=$!
sleep 1

"$SCRIPTS_DIR/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -icount shift=0,align=off,sleep=off \
    -device virtmcu-clock,mode=slaved-icount,node=1,router=$ENDPOINT \
    -device netdev -device netdev -netdev zenoh,node=1,id=n1,router=$ENDPOINT \
    -nographic -monitor none > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

# Wait for clock queryable
CLOCK_TOPIC="sim/clock/advance/1"
deadline=$(( $(date +%s) + 15 ))
while (( $(date +%s) < deadline )); do
    if python3 -c "import zenoh, sys, struct; c=zenoh.Config(); c.insert_json5('connect/endpoints', '[\"$ENDPOINT\"]'); c.insert_json5('scouting/multicast/enabled', 'false'); s=zenoh.open(c); r=list(s.get('$CLOCK_TOPIC', payload=b'ping', timeout=0.5)); s.close(); sys.exit(0 if r else 1)" 2>/dev/null; then
        break
    fi
    sleep 0.25
done

# Functional test
python3 - "$CLOCK_TOPIC" "$ENDPOINT" <<'PY_EOF'
import sys, struct, time, zenoh
CLOCK_TOPIC = sys.argv[1]
ROUTER = sys.argv[2]
NETDEV_TOPIC = "sim/eth/frame/1/rx"
DELIVERY_VTIME_NS = 500_000
FRAME = b'\xff' * 14
packet = vproto.ZenohFrameHeader(DELIVERY_VTIME_NS, 0, len(FRAME).pack()) + FRAME
c = zenoh.Config()
c.insert_json5("connect/endpoints", f'["{ROUTER}"]')
c.insert_json5("scouting/multicast/enabled", "false")
session = zenoh.open(c)
pub = session.declare_publisher(NETDEV_TOPIC)
pub.put(packet)
time.sleep(0.1)
replies = list(session.get(CLOCK_TOPIC, payload=vproto.ClockAdvanceReq(1000000, 0, 0).pack(), timeout=5.0))
if not replies: sys.exit(1)
session.close()
import sys
sys.stdout.write("PASS\n")
PY_EOF

echo "=== netdev test PASSED ==="
