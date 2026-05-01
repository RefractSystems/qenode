#!/usr/bin/env bash
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

SOCK_PATH="/tmp/virtmcu-stress-$$.sock"
DTB_PATH="/tmp/virtmcu-stress-$$.dtb"
DTS_PATH="/tmp/virtmcu-stress-$$.dts"
ELF_PATH="/tmp/virtmcu-stress-$$.elf"
LD_PATH="/tmp/virtmcu-stress-$$.ld"
QEMU_LOG="/tmp/virtmcu-stress-qemu-$$.log"

cleanup() {
    kill "${QEMU_PID:-}"    2>/dev/null || true
    kill "${ADAPTER_PID:-}" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$LD_PATH"
}
trap cleanup EXIT

echo "[stress] Building stress adapter..."
g++ -O3 "$SCRIPT_DIR/stress_adapter.cpp" -o "$SCRIPT_DIR/stress_adapter"

echo "[stress] Building firmware..."
cat > "$LD_PATH" <<'EOF'
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text*) }
}
EOF
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$SCRIPT_DIR/stress_firmware.S" -o "$ELF_PATH"

echo "[stress] Compiling device tree..."
cat > "$DTS_PATH" <<EOF
/dts-v1/;
/ {
    model = "virtmcu-stress-test";
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
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
            memory = <0x01>;
        };
    };
    uart0@9000000 {
        compatible = "pl011";
        reg = <0x0 0x09000000 0x0 0x1000>;
        chardev = <0x00>;
    };
    bridge@50000000 {
        compatible = "mmio-socket-bridge";
        reg = <0x0 0x70000000 0x0 0x1000>;
        socket-path = "$SOCK_PATH";
        region-size = <0x1000>;
    };
};
EOF
dtc -I dts -O dtb -o "$DTB_PATH" "$DTS_PATH"

echo "[stress] Starting stress adapter..."
"$SCRIPT_DIR/stress_adapter" "$SOCK_PATH" &
ADAPTER_PID=$!

for _ in $(seq 1 50); do [ -S "$SOCK_PATH" ] && break; sleep 0.1; done

echo "[stress] Starting QEMU (this will take a while)..."
# We expect 16M iterations * 2 MMIOs = 32M MMIO operations.
# Each op takes ~50-100us, so 32M * 50us = 1600s = 26 minutes.
# That's too long for a smoke test. Let's reduce firmware iterations to 100k.
# Actually, I'll just change it in the S file now.

sed -i 's/0x1000000/0xF4240/' "$SCRIPT_DIR/stress_firmware.S" # 1,000,000 iterations
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$SCRIPT_DIR/stress_firmware.S" -o "$ELF_PATH"

START_TIME=$(date +%s)
"$RUN_SH" --dtb "$DTB_PATH" --kernel "$ELF_PATH" -nographic -monitor none > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

echo "[stress] Waiting for OK..."
PASSED=false
for _ in $(seq 1 300); do # 30 seconds timeout
    if grep -q "OK" "$QEMU_LOG"; then
        PASSED=true
        break
    fi
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then break; fi
    sleep 0.1
done

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

if [ "$PASSED" = true ]; then
    echo "[stress] PASSED in ${DURATION}s"
    # Calculate MIPS or MMIOs/sec
    if [ "$DURATION" -gt 0 ]; then
        RATE=$((2000000 / DURATION))
        echo "[stress] Throughput: ~$RATE MMIOs/sec"
    else
        echo "[stress] Throughput: >2000000 MMIOs/sec"
    fi
else
    echo "[stress] FAILED"
    cat "$QEMU_LOG"
    exit 1
fi
