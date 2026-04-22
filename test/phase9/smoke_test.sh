#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

SOCK_PATH="/tmp/virtmcu-systemc-p9-$$.sock"
ADAPTER_LOG="/tmp/virtmcu-adapter-p9-$$.log"
QEMU_LOG="/tmp/virtmcu-qemu-p9-$$.log"
DTB_PATH="/tmp/virtmcu-p9-$$.dtb"
DTS_PATH="/tmp/virtmcu-p9-$$.dts"
ELF_PATH="/tmp/virtmcu-p9-$$.elf"
ASM_PATH="/tmp/virtmcu-p9-$$.S"
LD_PATH="/tmp/virtmcu-p9-$$.ld"
ZENOH_TX_PY="/tmp/virtmcu-zenoh-tx-$$.py"

ADAPTER_PID=0
QEMU_PID=0
ROUTER_PID=0

cleanup() {
    [[ $QEMU_PID -ne 0 ]] && kill "$QEMU_PID" 2>/dev/null || true
    [[ $ADAPTER_PID -ne 0 ]] && kill "$ADAPTER_PID" 2>/dev/null || true
    [[ $ROUTER_PID -ne 0 ]] && kill "$ROUTER_PID" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$ADAPTER_LOG" "$QEMU_LOG" \
          "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$ASM_PATH" "$LD_PATH" "$ZENOH_TX_PY"
}
trap cleanup EXIT

echo "[phase9] Starting Zenoh Router..."
ZENOH_PORT=$(( 7447 + (RANDOM % 1000) ))
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "tcp/127.0.0.1:$ZENOH_PORT" &
ROUTER_PID=$!
sleep 1

export ZENOH_CONNECT="tcp/127.0.0.1:$ZENOH_PORT"
export ZENOH_MULTICAST_SCOUTING="false"

echo "[phase9] Building SystemC adapter..."
make -C "$WORKSPACE_DIR/tools/systemc_adapter" > /dev/null

# ==============================================================================
# TEST 1: RegisterFile MMIO + IRQ
# ==============================================================================
echo "[phase9] --- TEST 1: RegisterFile MMIO + IRQ ---"

echo "[phase9] Starting SystemC adapter (standalone)..."
"$WORKSPACE_DIR/tools/systemc_adapter/build/adapter" "$SOCK_PATH" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

for _ in $(seq 1 50); do [ -S "$SOCK_PATH" ] && break; sleep 0.1; done

cat > "$LD_PATH" <<'LD_EOF'
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text*) }
    .data : { *(.data*) }
}
LD_EOF

cat > "$ASM_PATH" <<'ASM_EOF'
.equ UART0_DR, 0x09000000
.equ BRIDGE_BASE, 0x50000000
.equ GICD_BASE, 0x08000000
.equ GICD_ISPENDR1, (GICD_BASE + 0x204)

.global _start
_start:
    /* Trigger IRQ 0 on bridge (SPI 32 in QEMU/GIC) via reg 255 */
    ldr r0, =BRIDGE_BASE
    add r0, r0, #255*4
    mov r1, #1
    str r1, [r0]

    /* Poll GICD_ISPENDR1 (bit 0 for IRQ 32) */
    ldr r0, =GICD_ISPENDR1
wait_irq:
    ldr r1, [r0]
    tst r1, #1
    beq wait_irq

    /* Send "REG-OK" to UART */
    ldr r0, =UART0_DR
    mov r1, #'R'
    str r1, [r0]
    mov r1, #'E'
    str r1, [r0]
    mov r1, #'G'
    str r1, [r0]
    mov r1, #'-'
    str r1, [r0]
    mov r1, #'O'
    str r1, [r0]
    mov r1, #'K'
    str r1, [r0]

loop:
    nop
    b loop
ASM_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$ASM_PATH" -o "$ELF_PATH"

cat > "$DTS_PATH" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-phase9-test";
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;
    qemu_sysmem: qemu_sysmem { compatible = "qemu:system-memory"; phandle = <0x01>; };
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
    gic: interrupt-controller@8000000 {
        compatible = "arm_gic";
        #interrupt-cells = <3>;
        interrupt-controller;
        reg = <0x0 0x08000000 0x0 0x1000>, <0x0 0x08010000 0x0 0x1000>;
        num-irq = <64>;
    };
    uart0: pl011@9000000 {
        compatible = "pl011";
        reg = <0x0 0x09000000 0x0 0x1000>;
        chardev = <0x00>;
    };
    bridge: bridge@50000000 {
        compatible = "mmio-socket-bridge";
        reg = <0x0 0x50000000 0x0 0x1000>;
        socket-path = "SOCK_PLACEHOLDER";
        region-size = <0x1000>;
        interrupt-parent = <&gic>;
        interrupts = <0 0 4>; 
    };
};
DTS_EOF

sed -i "s|SOCK_PLACEHOLDER|$SOCK_PATH|" "$DTS_PATH"
dtc -I dts -O dtb -o "$DTB_PATH" "$DTS_PATH"

"$RUN_SH" --dtb "$DTB_PATH" --kernel "$ELF_PATH" -nographic -monitor none \
    -icount shift=0,align=off,sleep=off > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

echo "[phase9] Waiting for TEST 1 results..."
for _ in $(seq 1 100); do
    if grep -q "REG-OK" "$QEMU_LOG" 2>/dev/null; then
        echo "[phase9] TEST 1 SUCCESS!"
        kill "$QEMU_PID" "$ADAPTER_PID" 2>/dev/null || true
        for _ in $(seq 1 50); do
            if ! kill -0 "$QEMU_PID" 2>/dev/null && ! kill -0 "$ADAPTER_PID" 2>/dev/null; then break; fi
            sleep 0.1
        done
        break
    fi
    sleep 0.1
done
grep -q "REG-OK" "$QEMU_LOG" || { 
    echo "TEST 1 FAILED"
    echo "--- QEMU LOG ---"
    cat "$QEMU_LOG"
    echo "--- ADAPTER LOG ---"
    cat "$ADAPTER_LOG"
    exit 1 
}

# ==============================================================================
# TEST 2: CAN Controller Zenoh RX → IRQ
# ==============================================================================
echo "[phase9] --- TEST 2: CAN Zenoh RX → IRQ ---"
rm -f "$SOCK_PATH" "$QEMU_LOG"

echo "[phase9] Starting SystemC adapter (node=p9-test)..."
"$WORKSPACE_DIR/tools/systemc_adapter/build/adapter" "$SOCK_PATH" "p9-test" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

for _ in $(seq 1 50); do [ -S "$SOCK_PATH" ] && break; sleep 0.1; done

cat > "$ASM_PATH" <<'ASM_EOF'
.equ UART0_DR, 0x09000000
.equ CAN_BASE, 0x50000000
.equ GICD_BASE, 0x08000000
.equ GICD_ISPENDR1, (GICD_BASE + 0x204)

.global _start
_start:
    /* Poll GICD_ISPENDR1 (bit 0 for IRQ 32) */
    ldr r0, =GICD_ISPENDR1
wait_irq:
    ldr r1, [r0]
    tst r1, #1
    beq wait_irq

    /* Verify rx_id at 0x10 */
    ldr r0, =CAN_BASE
    ldr r1, [r0, #0x10]
    ldr r2, =0x123
    cmp r1, r2
    bne fail

    /* Send "CAN-OK" to UART */
    ldr r0, =UART0_DR
    mov r1, #'C'
    str r1, [r0]
    mov r1, #'A'
    str r1, [r0]
    mov r1, #'N'
    str r1, [r0]
    mov r1, #'-'
    str r1, [r0]
    mov r1, #'O'
    str r1, [r0]
    mov r1, #'K'
    str r1, [r0]
    b done

fail:
    ldr r0, =UART0_DR
    mov r1, #'F'
    str r1, [r0]
    mov r1, #'A'
    str r1, [r0]
    mov r1, #'I'
    str r1, [r0]
    mov r1, #'L'
    str r1, [r0]

done:
loop:
    nop
    b loop
ASM_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$ASM_PATH" -o "$ELF_PATH"

"$RUN_SH" --dtb "$DTB_PATH" --kernel "$ELF_PATH" -nographic -monitor none \
    -icount shift=0,align=off,sleep=off > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

# Python script to send Zenoh frame
cat > "$ZENOH_TX_PY" <<'PY_EOF'
import zenoh
import struct
import time

session = zenoh.open(zenoh.Config())
time.sleep(2) # Give adapter time to declare subscriber
pub = session.declare_publisher("sim/systemc/frame/p9-test/rx")

# CanWireFrame: vtime(8), size(4), id(4), data(4)
# Stamp with a future vtime (e.g. 1ms)
payload = struct.pack("<QIII", 1000000, 8, 0x123, 0x456)
pub.put(payload)
time.sleep(0.5)
session.close()
PY_EOF

echo "[phase9] Injecting Zenoh CAN frame..."
python3 "$ZENOH_TX_PY"

echo "[phase9] Waiting for TEST 2 results..."
for _ in $(seq 1 100); do
    if grep -q "CAN-OK" "$QEMU_LOG" 2>/dev/null; then
        echo "[phase9] TEST 2 SUCCESS!"
        exit 0
    fi
    if grep -q "FAIL" "$QEMU_LOG" 2>/dev/null; then
        echo "[phase9] TEST 2 FAILED (ID mismatch)"
        exit 1
    fi
    sleep 0.1
done

echo "[phase9] TEST 2 TIMEOUT. Logs:"
echo "--- QEMU LOG ---"
cat "$QEMU_LOG"
echo "--- ADAPTER LOG ---"
cat "$ADAPTER_LOG"
exit 1
