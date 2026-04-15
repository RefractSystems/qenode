#!/usr/bin/env bash
# test/phase7/smoke_test.sh — Phase 7 smoke test: zenoh-clock suspend & icount modes.
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
test/phase7/smoke_test.sh — Phase 7 smoke test: zenoh-clock suspend & icount modes.
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_XXXXXX)"
QEMU_PID=""
ROUTER_PID=""

cleanup() {
    [[ -n "${QEMU_PID:-}" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    [[ -n "${ROUTER_PID:-}" ]] && kill -9 "$ROUTER_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

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

# Helper to wait for queryable
wait_for_queryable() {
    local topic="$1"
    local deadline=$(( $(date +%s) + 15 ))
    while (( $(date +%s) < deadline )); do
        if python3 -c "import zenoh, sys, struct; c=zenoh.Config(); c.insert_json5('connect/endpoints', '[\"tcp/127.0.0.1:7447\"]'); c.insert_json5('scouting/multicast/enabled', 'false'); s=zenoh.open(c); r=list(s.get('$topic', payload=struct.pack('<QQ', 0, 0), timeout=0.5)); s.close(); sys.exit(0 if r else 1)" 2>/dev/null; then
            return 0
        fi
        sleep 0.25
    done
    return 1
}

# Run: suspend mode
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" &
ROUTER_PID=$!
sleep 1
"$WORKSPACE_DIR/scripts/run.sh" --dtb "$TMPDIR_LOCAL/dummy.dtb" -kernel "$TMPDIR_LOCAL/firmware.elf" -device zenoh-clock,mode=suspend,node=0,router=tcp/127.0.0.1:7447 -nographic -monitor none > "$TMPDIR_LOCAL/qemu_suspend.log" 2>&1 &
QEMU_PID=$!
wait_for_queryable "sim/clock/advance/0"
python3 "$WORKSPACE_DIR/test/phase7/test_clock.py"
kill -9 "$QEMU_PID" "$ROUTER_PID" 2>/dev/null || true
QEMU_PID=""
ROUTER_PID=""

# Run: icount mode
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" &
ROUTER_PID=$!
sleep 1
"$WORKSPACE_DIR/scripts/run.sh" --dtb "$TMPDIR_LOCAL/dummy.dtb" -kernel "$TMPDIR_LOCAL/firmware.elf" -icount shift=0,align=off,sleep=off -device zenoh-clock,mode=icount,node=0,router=tcp/127.0.0.1:7447 -nographic -monitor none > "$TMPDIR_LOCAL/qemu_icount.log" 2>&1 &
QEMU_PID=$!
wait_for_queryable "sim/clock/advance/0"
python3 "$WORKSPACE_DIR/test/phase7/test_clock.py"
kill -9 "$QEMU_PID" "$ROUTER_PID" 2>/dev/null || true

echo "=== Phase 7 smoke test PASSED ==="
