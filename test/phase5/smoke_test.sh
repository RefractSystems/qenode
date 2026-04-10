#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 5 - Path A)
#
# Validates the mmio-socket-bridge and tools/systemc_adapter by writing
# to the register file and reading the value back using QEMU's monitor.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

SOCK_PATH="/tmp/virtmcu-systemc.sock"
QMP_SOCK="/tmp/qmp-phase5.sock"

echo "Building SystemC adapter..."
make -C "$WORKSPACE_DIR/tools/systemc_adapter" > /dev/null

echo "Starting SystemC adapter..."
rm -f "$SOCK_PATH"
"$WORKSPACE_DIR/tools/systemc_adapter/build/adapter" "$SOCK_PATH" > adapter.log 2>&1 &
ADAPTER_PID=$!

# Wait for socket
sleep 1

# Generate a dummy DTB to satisfy arm-generic-fdt requirement
cat <<EOF > /tmp/phase5_dummy.dts
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
EOF
dtc -I dts -O dtb -o /tmp/phase5_dummy.dtb /tmp/phase5_dummy.dts

cat <<EOF > /tmp/phase5_firmware.S
.global _start
_start:
    ldr r0, =0x50000000
    ldr r1, =0xdeadbeef
    str r1, [r0, #4]
    ldr r2, [r0, #4]
loop:
    wfi
    b loop
EOF
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -g -T "$WORKSPACE_DIR/test/phase1/linker.ld" /tmp/phase5_firmware.S -o /tmp/phase5_firmware.elf

echo "Starting QEMU..."
"$RUN_SH" --dtb /tmp/phase5_dummy.dtb \
    -kernel /tmp/phase5_firmware.elf \
    -device mmio-socket-bridge,socket-path="$SOCK_PATH",region-size=4096,base-addr=0x50000000 \
    -nographic \
    -monitor none \
    -qmp unix:"$QMP_SOCK",server,nowait \
    > qemu_phase5.log 2>&1 &
QEMU_PID=$!

sleep 2

# Check adapter log for the read and write
if grep -q "Wrote deadbeef to reg 1" adapter.log && grep -q "Read deadbeef from reg 1" adapter.log; then
    echo "Phase 5 smoke test: PASSED"
    RET=0
else
    echo "Phase 5 smoke test: FAILED"
    cat adapter.log
    cat qemu_phase5.log
    RET=1
fi

kill $QEMU_PID 2>/dev/null || true
kill $ADAPTER_PID 2>/dev/null || true
rm -f "$SOCK_PATH" "$QMP_SOCK" /tmp/phase5_dummy.* /tmp/test_phase5.py

exit $RET
