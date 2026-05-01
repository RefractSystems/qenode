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

SOCK_PATH="/tmp/virtmcu-systemc-p9-$$.sock"
ADAPTER_LOG="/tmp/virtmcu-adapter-p9-$$.log"
QEMU_LOG="/tmp/virtmcu-qemu-p9-$$.log"
DTB_PATH="/tmp/virtmcu-p9-$$.dtb"
DTS_PATH="/tmp/virtmcu-p9-$$.dts"
ELF_PATH="/tmp/virtmcu-p9-$$.elf"
ASM_PATH="/tmp/virtmcu-p9-$$.S"
LD_PATH="/tmp/virtmcu-p9-$$.ld"
ZENOH_TX_PY="/tmp/transport-zenoh-tx-$$.py"

ADAPTER_PID=0
QEMU_PID=0
ROUTER_PID=0

# Under ASan/UBSan, QEMU executes TCG blocks significantly slower.
# Scale polling iteration counts from the stall-timeout environment variable
# (CI: 120000 ms; ASan CI: 300000 ms; default: 5000 ms).
# Each iteration sleeps 0.1 s, so divide by 100 to get iteration count.
# Clamp to a minimum of 100 (10 s) and a maximum of 3000 (300 s).
_stall_ms="${VIRTMCU_STALL_TIMEOUT_MS:-5000}"
_poll_iters=$(( _stall_ms / 100 ))
(( _poll_iters < 100  )) && _poll_iters=100
(( _poll_iters > 3000 )) && _poll_iters=3000
POLL_ITERS=$_poll_iters

cleanup() {
    [[ $QEMU_PID -ne 0 ]] && kill "$QEMU_PID" 2>/dev/null || true
    [[ $ADAPTER_PID -ne 0 ]] && kill "$ADAPTER_PID" 2>/dev/null || true
    [[ $ROUTER_PID -ne 0 ]] && kill "$ROUTER_PID" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$ADAPTER_LOG" "$QEMU_LOG" \
          "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$ASM_PATH" "$LD_PATH" "$ZENOH_TX_PY"
}
trap cleanup EXIT

echo "[ftrt_timing] Starting Zenoh Router..."
ZENOH_ENDPOINT=$(python3 "$SCRIPTS_DIR/get-free-port.py" --endpoint --proto "tcp/")
python3 -u "$WORKSPACE_DIR/tests/zenoh_router_persistent.py" "$ZENOH_ENDPOINT" &
ROUTER_PID=$!
sleep 1

export ZENOH_CONNECT="$ZENOH_ENDPOINT"
export ZENOH_MULTICAST_SCOUTING="false"
export WORKSPACE_DIR="$WORKSPACE_DIR"

echo "[ftrt_timing] Building SystemC adapter..."
make -C "$TOOLS_DIR/systemc_adapter" > /dev/null

# ==============================================================================
# TEST 1: RegisterFile MMIO + IRQ
# ==============================================================================
echo "[ftrt_timing] --- TEST 1: RegisterFile MMIO + IRQ ---"

echo "[ftrt_timing] Starting SystemC adapter (standalone)..."
"$TOOLS_DIR/systemc_adapter/build/adapter" "$SOCK_PATH" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

for _ in $(seq 1 $POLL_ITERS); do [ -S "$SOCK_PATH" ] && break; sleep 0.1; done

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
    model = "virtmcu-ftrt_timing-test";
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

echo "[ftrt_timing] Waiting for TEST 1 results..."
for _ in $(seq 1 $POLL_ITERS); do
    if grep -q "REG-OK" "$QEMU_LOG" 2>/dev/null; then
        echo "[ftrt_timing] TEST 1 SUCCESS!"
        kill "$QEMU_PID" "$ADAPTER_PID" 2>/dev/null || true
        for _ in $(seq 1 $POLL_ITERS); do
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
echo "[ftrt_timing] --- TEST 2: CAN Zenoh RX → IRQ ---"
rm -f "$SOCK_PATH" "$QEMU_LOG"

echo "[ftrt_timing] Starting SystemC adapter (node=p9-test)..."
"$TOOLS_DIR/systemc_adapter/build/adapter" "$SOCK_PATH" "p9-test" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

for _ in $(seq 1 $POLL_ITERS); do [ -S "$SOCK_PATH" ] && break; sleep 0.1; done

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
import time
import os
import sys

if "WORKSPACE_DIR" in os.environ:
    sys.path.append(os.path.join(os.environ["WORKSPACE_DIR"], "tools"))
import vproto

config = zenoh.Config()
if "ZENOH_CONNECT" in os.environ:
    config.insert_json5("connect/endpoints", f'["{os.environ["ZENOH_CONNECT"]}"]')

session = zenoh.open(config)
pub = session.declare_publisher("sim/systemc/frame/p9-test/rx")

# CanWireFrame: ZenohFrameHeader(24), id(4), data(4)
# Under ASan, QEMU initialization and Zenoh discovery takes unpredictable wall-clock time.
# Send frames with exponentially/linearly increasing vtime (1s to 50s virtual time)
# so the adapter receives at least one frame in its "future".
sys.path.append(os.path.join(os.environ["WORKSPACE_DIR"], "hw/rust/common/virtmcu-api/python"))
import vproto
for i in range(1, 50):
    vtime_ns = i * 1000000000 # i seconds
    hdr = vproto.ZenohFrameHeader(vtime_ns, 0, 8).pack()
    payload = hdr + (0x123).to_bytes(4, "little") + (0x456).to_bytes(4, "little")
    pub.put(payload)
    time.sleep(0.5)

session.close()
PY_EOF

echo "[ftrt_timing] Injecting Zenoh CAN frame..."
python3 "$ZENOH_TX_PY" &
PY_PID=$!

echo "[ftrt_timing] Waiting for TEST 2 results..."
for _ in $(seq 1 $POLL_ITERS); do
    if grep -q "CAN-OK" "$QEMU_LOG" 2>/dev/null; then
        echo "[ftrt_timing] TEST 2 SUCCESS!"
        kill $PY_PID 2>/dev/null || true
        exit 0
    fi
    if grep -q "FAIL" "$QEMU_LOG" 2>/dev/null; then
        echo "[ftrt_timing] TEST 2 FAILED (ID mismatch)"
        kill $PY_PID 2>/dev/null || true
        exit 1
    fi
    sleep 0.1
done

kill $PY_PID 2>/dev/null || true
echo "[ftrt_timing] TEST 2 TIMEOUT. Logs:"
echo "--- QEMU LOG ---"
cat "$QEMU_LOG"
echo "--- ADAPTER LOG ---"
cat "$ADAPTER_LOG"
exit 1

