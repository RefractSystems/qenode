#!/usr/bin/env bash
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_stall_XXXXXX)"
QEMU_PID=""
ROUTER_PID=""

cleanup() {
    [[ -n "${QEMU_PID:-}" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    [[ -n "${ROUTER_PID:-}" ]] && kill -9 "$ROUTER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Minimal firmware
cat > "$TMPDIR_LOCAL/linker.ld" <<'LD_EOF'
SECTIONS { . = 0x40000000; .text : { *(.text) } }
LD_EOF
cat > "$TMPDIR_LOCAL/firmware.S" <<'ASM_EOF'
.global _start
_start: loop: nop; b loop
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

python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" &
ROUTER_PID=$!
sleep 1

# Start test queryable
python3 -u "$WORKSPACE_DIR/test/phase7/clock_stall_test.py" &
PYTHON_PID=$!
sleep 1

echo "Starting QEMU..."
"$WORKSPACE_DIR/scripts/run.sh" --dtb "$TMPDIR_LOCAL/dummy.dtb" -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -icount shift=0,align=off,sleep=off \
    -device zenoh-clock,mode=slaved-suspend,node=0,router=tcp/127.0.0.1:7447,stall-timeout=2000 \
    -nographic -monitor none > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

sleep 4

if kill -0 $QEMU_PID 2>/dev/null; then
    echo "QEMU failed to abort on clock stall timeout!"
    cat "$TMPDIR_LOCAL/qemu.log"
    kill -9 $PYTHON_PID
    exit 1
fi

echo "QEMU successfully exited due to stall. Checking logs..."
cat "$TMPDIR_LOCAL/qemu.log"
if ! grep -q "Timeout waiting for clock quantum boundary" "$TMPDIR_LOCAL/qemu.log"; then
    echo "QEMU exited, but log does not contain expected timeout error message."
    kill -9 $PYTHON_PID
    exit 1
fi

echo "=== Phase 7 Clock Stall Test PASSED ==="
kill -9 $PYTHON_PID || true
