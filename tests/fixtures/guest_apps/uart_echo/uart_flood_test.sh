#!/usr/bin/env bash
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
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
TMPDIR_LOCAL="$(mktemp -d /tmp/uart_echo_uart_XXXXXX)"
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

# Minimal DTB with PL011 chardev
cat > "$TMPDIR_LOCAL/dummy.dts" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-test"; compatible = "arm,generic-fdt"; #address-cells = <2>; #size-cells = <2>;
    qemu_sysmem: qemu_sysmem { compatible = "qemu:system-memory"; phandle = <0x01>; };
    chosen {};
    memory@40000000 { compatible = "qemu-memory-region"; qemu,ram = <0x01>; container = <0x01>; reg = <0x0 0x40000000 0x0 0x10000000>; };
    cpus { #address-cells = <1>; #size-cells = <0>; cpu@0 { device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; }; };
    uart@9000000 { compatible = "arm,pl011", "arm,primecell"; reg = <0x0 0x9000000 0x0 0x1000>; interrupts = <0x0 0x1 0x4>; };
};
DTS_EOF
dtc -I dts -O dtb -o "$TMPDIR_LOCAL/dummy.dtb" "$TMPDIR_LOCAL/dummy.dts"

# Find a free port
ENDPOINT=$(python3 "$SCRIPTS_DIR/get-free-port.py" --endpoint --proto "tcp/")

python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "$ENDPOINT" &
ROUTER_PID=$!
sleep 1

"$SCRIPTS_DIR/run.sh" --dtb "$TMPDIR_LOCAL/dummy.dtb" -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -chardev "zenoh,id=uart0,node=0,router=$ENDPOINT" \
    -serial chardev:uart0 \
    -nographic -monitor none > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

sleep 2

python3 "$WORKSPACE_DIR/tests/fixtures/guest_apps/uart_echo/uart_flood_test.py" "$ENDPOINT"

if ! kill -0 $QEMU_PID 2>/dev/null; then
    echo "QEMU crashed during UART flood test!"
    cat "$TMPDIR_LOCAL/qemu.log"
    exit 1
fi

echo "=== UART Flood Test PASSED (No Crash) ==="
