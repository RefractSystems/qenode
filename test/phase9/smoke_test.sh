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

cleanup() {
    kill "$QEMU_PID"    2>/dev/null || true
    kill "$ADAPTER_PID" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$ADAPTER_LOG" "$QEMU_LOG" \
          "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$ASM_PATH" "$LD_PATH"
}
trap cleanup EXIT

echo "[phase9] Building SystemC adapter..."
make -C "$WORKSPACE_DIR/tools/systemc_adapter" > /dev/null

echo "[phase9] Starting SystemC adapter..."
"$WORKSPACE_DIR/tools/systemc_adapter/build/adapter" "$SOCK_PATH" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

for _ in $(seq 1 50); do [ -S "$SOCK_PATH" ] && break; sleep 0.1; done

# ── Firmware ────────────────────────────────────────────────────────────────
# This firmware:
# 1. Prints a message.
# 2. Triggers an IRQ by writing to the bridge.
# 3. Since we don't have a full GIC setup in this minimal ASM, we will 
#    just verify the adapter side for now, OR we can try a minimal GIC setup.
# Actually, let's just verify the adapter saw the write and sent the IRQ.
# To truly test IRQ in QEMU, we'd need to catch it.
# Let's use a simple polling of the GIC Pending register if possible.

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
.equ GICD_ISPENDR0, (GICD_BASE + 0x200)

.global _start
_start:
    /* 1. Send "START" to UART */
    ldr r0, =UART0_DR
    mov r1, #'S'
    str r1, [r0]
    mov r1, #'T'
    str r1, [r0]

    /* 2. Trigger IRQ 0 on bridge (SPI 32 in QEMU/GIC) */
    /* Bridge IRQ 0 -> GIC SPI 32 (ID 32+32=64? No, SPI starts at 32) */
    ldr r0, =BRIDGE_BASE
    add r0, r0, #255*4
    mov r1, #1
    str r1, [r0]

    /* 3. Poll GICD_ISPENDR to see if IRQ is pending */
    /* IRQ 32 is bit 0 of ISPENDR1 (0x204) */
    ldr r0, =(GICD_BASE + 0x204)
wait_irq:
    ldr r1, [r0]
    tst r1, #1
    beq wait_irq

    /* 4. Send "DONE" to UART */
    ldr r0, =UART0_DR
    mov r1, #'D'
    str r1, [r0]
    mov r1, #'O'
    str r1, [r0]
    mov r1, #'N'
    str r1, [r0]
    mov r1, #'E'
    str r1, [r0]

loop:
    wfi
    b loop
ASM_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$ASM_PATH" -o "$ELF_PATH"

# ── Device Tree ─────────────────────────────────────────────────────────────
cat > "$DTS_PATH" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-phase9-test";
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;

    qemu_sysmem: qemu_sysmem {
        compatible = "qemu:system-memory";
        phandle = <0x01>;
    };

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

    gic: interrupt-controller@8000000 {
        compatible = "arm_gic";
        #interrupt-cells = <3>;
        interrupt-controller;
        reg = <0x0 0x08000000 0x0 0x1000>,
              <0x0 0x08010000 0x0 0x1000>;
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
        /* SPI 0 is ID 32.  <type=0(SPI) ID-32 edge/level> */
        interrupts = <0 0 4>, <0 1 4>, <0 2 4>; 
    };
};
DTS_EOF

sed -i "s|SOCK_PLACEHOLDER|$SOCK_PATH|" "$DTS_PATH"
dtc -I dts -O dtb -o "$DTB_PATH" "$DTS_PATH"

echo "[phase9] Starting QEMU..."
"$RUN_SH" --dtb "$DTB_PATH" --kernel "$ELF_PATH" -nographic -monitor none > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

echo "[phase9] Waiting for results..."
for _ in $(seq 1 100); do
    if grep -q "DONE" "$QEMU_LOG" 2>/dev/null; then
        echo "[phase9] SUCCESS: IRQ detected by firmware!"
        exit 0
    fi
    sleep 0.1
done

echo "[phase9] FAILED: Timed out waiting for IRQ. Logs:"
echo "--- QEMU LOG ---"
cat "$QEMU_LOG"
echo "--- ADAPTER LOG ---"
cat "$ADAPTER_LOG"
exit 1
