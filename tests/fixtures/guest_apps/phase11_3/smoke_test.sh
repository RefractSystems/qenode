#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 11.3 — Path B: Remote Port Co-Simulation)
#
# What this tests:
#   Validates the full Path B co-simulation pipeline via AMD/Xilinx Remote Port:
#     QEMU firmware write/read → remote-port-bridge (QOM) → Unix socket →
#     SystemC adapter (rp_adapter) using libsystemctlm-soc → TLM-2.0 → RegisterFile
# ==============================================================================

set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
==============================================================================
smoke_test.sh (Phase 11.3 — Path B: Remote Port Co-Simulation)

What this tests:
  Validates the full Path B co-simulation pipeline via AMD/Xilinx Remote Port:
    QEMU firmware write/read → remote-port-bridge (QOM) → Unix socket →
    SystemC adapter (rp_adapter) using libsystemctlm-soc → TLM-2.0 → RegisterFile
==============================================================================
TEST_DOC_BLOCK
echo "=============================================================================="


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

SOCK_PATH="/tmp/virtmcu-rp-$$.sock"
ADAPTER_LOG="/tmp/virtmcu-rp-adapter-$$.log"
QEMU_LOG="/tmp/virtmcu-rp-qemu-$$.log"
DTB_PATH="/tmp/virtmcu-rp-$$.dtb"
DTS_PATH="/tmp/virtmcu-rp-$$.dts"
ELF_PATH="/tmp/virtmcu-rp-$$.elf"
ASM_PATH="/tmp/virtmcu-rp-$$.S"
LD_PATH="/tmp/virtmcu-rp-$$.ld"

cleanup() {
    echo "[phase11_3] Cleaning up..."
    kill "${QEMU_PID:-}" 2>/dev/null || true
    kill "${ADAPTER_PID:-}" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$ASM_PATH" "$LD_PATH" "$ELF_PATH" "$DTB_PATH" "$DTS_PATH" "$ADAPTER_LOG" "$QEMU_LOG"
}
trap cleanup EXIT

echo "[phase11_3] Building SystemC Remote Port adapter..."
ADAPTER_BUILD_DIR="$WORKSPACE_DIR/tools/systemc_adapter/build"
if [ ! -f "$ADAPTER_BUILD_DIR/CMakeCache.txt" ]; then
    echo "[phase11_3] CMake not yet configured — running cmake..."
    cmake -S "$WORKSPACE_DIR/tools/systemc_adapter" -B "$ADAPTER_BUILD_DIR" -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5 -DGIT_EXECUTABLE="$(which git)" > /dev/null 2>&1
fi
make -C "$ADAPTER_BUILD_DIR" rp_adapter > /dev/null 2>&1

echo "[phase11_3] Starting SystemC adapter on $SOCK_PATH..."
"$WORKSPACE_DIR/tools/systemc_adapter/build/rp_adapter" "unix:$SOCK_PATH" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

# Wait for adapter to create the socket
sleep 1

echo "[phase11_3] Compiling ARM firmware..."
cat << 'ASM_EOF' > "$ASM_PATH"
.section .text
.global _start
_start:
    /* Write 0xdeadbeef to bridge at 0x60000000 */
    ldr r0, =0x60000000
    ldr r1, =0xdeadbeef
    str r1, [r0]

    /* Read it back */
    ldr r2, [r0]
    
    /* Write another value */
    ldr r1, =0x11223344
    str r1, [r0, #4]

    /* WFI loop */
1:  nop
    b 1b
ASM_EOF

cat << 'LD_EOF' > "$LD_PATH"
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text) }
}
LD_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -mthumb -nostdlib -nostartfiles \
    -T "$LD_PATH" "$ASM_PATH" -o "$ELF_PATH"

echo "[phase11_3] Compiling device tree..."
cat << DTS_EOF > "$DTS_PATH"
/dts-v1/;
/ {
    #address-cells = <2>;
    #size-cells = <2>;
    model = "virtmcu-rp-test";
    compatible = "arm,generic-fdt";

    qemu_sysmem: qemu_sysmem {
        compatible = "qemu:system-memory";
        phandle = <1>;
    };

    cpus {
        #address-cells = <1>;
        #size-cells = <0>;
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
            memory = <1>;
        };
    };

    memory@40000000 {
        compatible = "qemu-memory-region";
        qemu,ram = <1>;
        container = <1>;
        reg = <0x0 0x40000000 0x0 0x8000000>;
    };

    bridge@60000000 {
        compatible = "remote-port-bridge";
        reg = <0x0 0x60000000 0x0 0x1000>;
        socket-path = "$SOCK_PATH";
        region-size = <0x1000>;
    };
};
DTS_EOF

dtc -I dts -O dtb -o "$DTB_PATH" "$DTS_PATH" 2>/dev/null

echo "[phase11_3] Starting QEMU..."
"$RUN_SH" \
    --dtb "$DTB_PATH" \
    -kernel "$ELF_PATH" \
    -nographic \
    -monitor none > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

echo "[phase11_3] Waiting for Remote Port transactions..."
timeout=10
while [ $timeout -gt 0 ]; do
    if grep -q "WRITE" "$ADAPTER_LOG" && grep -q "READ" "$ADAPTER_LOG"; then
        break
    fi
    sleep 1
    timeout=$((timeout - 1))
done

cat "$ADAPTER_LOG"

if [ $timeout -eq 0 ]; then
    echo "✗ Phase 11.3 smoke test FAILED: Did not find WRITE and READ in adapter log."
    exit 1
fi

if ! grep -q "efbeadde" "$ADAPTER_LOG"; then
    echo "✗ Phase 11.3 smoke test FAILED: Expected payload 'efbeadde' (little-endian deadbeef) not found in write log."
    exit 1
fi

echo "✓ Phase 11.3 smoke test PASSED: SystemC adapter received MMIO read/writes via AMD/Xilinx Remote Port."
exit 0
