#!/usr/bin/env bash
# ==============================================================================
# smoke_test.sh (Phase 5 — Path A: mmio-socket-bridge)
#
# What this tests:
#   Validates the full Path A co-simulation pipeline:
#     QEMU firmware write/read → mmio-socket-bridge (QOM) → Unix socket →
#     SystemC adapter (main.cpp) → TLM-2.0 b_transport → RegisterFile
#
# How it works:
#   1. Build the SystemC adapter.
#   2. Start the adapter (it listens on a Unix socket).
#   3. Compile a minimal ARM firmware that writes 0xdeadbeef to offset 0 of the
#      bridge MMIO region, then reads it back.
#   4. Start QEMU with the bridge device mapped at 0x50000000.
#   5. Assert that the adapter's log shows the expected write and read.
#
# Dependencies:
#   - arm-none-eabi-gcc  (cross-compiler)
#   - dtc                (device tree compiler)
#   - A built QEMU binary (via scripts/setup-qemu.sh)
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

SOCK_PATH="/tmp/virtmcu-systemc-$$.sock"
QMP_SOCK="/tmp/qmp-phase5-$$.sock"
ADAPTER_LOG="/tmp/virtmcu-adapter-$$.log"
QEMU_LOG="/tmp/virtmcu-qemu-phase5-$$.log"
DTB_PATH="/tmp/virtmcu-phase5-$$.dtb"
DTS_PATH="/tmp/virtmcu-phase5-$$.dts"
ELF_PATH="/tmp/virtmcu-phase5-$$.elf"
ASM_PATH="/tmp/virtmcu-phase5-$$.S"
LD_PATH="/tmp/virtmcu-phase5-$$.ld"

cleanup() {
    kill "${QEMU_PID:-}"    2>/dev/null || true
    kill "${ADAPTER_PID:-}" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$QMP_SOCK" "$ADAPTER_LOG" "$QEMU_LOG" \
          "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$ASM_PATH" "$LD_PATH"
}
trap cleanup EXIT

# ── 1. Build SystemC adapter ──────────────────────────────────────────────────
echo "[phase5] Building SystemC adapter..."
make -C "$WORKSPACE_DIR/tools/systemc_adapter" > /dev/null

# ── 1b. Standalone protocol test (no QEMU needed) ────────────────────────────
echo "[phase5] Running standalone protocol test..."
python3 "$SCRIPT_DIR/test_proto.py" \
    "$WORKSPACE_DIR/tools/systemc_adapter/build/adapter"
echo "[phase5] Protocol test: PASSED"

# ── 2. Start adapter ──────────────────────────────────────────────────────────
echo "[phase5] Starting SystemC adapter on $SOCK_PATH..."
"$WORKSPACE_DIR/tools/systemc_adapter/build/adapter" "$SOCK_PATH" > "$ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!

# Poll for the socket file (adapter calls bind() before printing its message)
for _ in $(seq 1 50); do
    [ -S "$SOCK_PATH" ] && break
    sleep 0.1
done
if [ ! -S "$SOCK_PATH" ]; then
    echo "[phase5] ERROR: adapter socket did not appear. Log:"
    cat "$ADAPTER_LOG"
    exit 1
fi

# ── 3. Build firmware ─────────────────────────────────────────────────────────
echo "[phase5] Compiling ARM firmware..."

# Self-contained linker script — no dependency on Phase 1 artifacts.
cat > "$LD_PATH" <<'EOF'
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text*) }
    .data : { *(.data*) }
    .bss  : { *(.bss*)  }
}
EOF

# Firmware: write 0xdeadbeef to offset 0 of the bridge, read it back, spin.
cat > "$ASM_PATH" <<'EOF'
.global _start
_start:
    ldr r0, =0x50000000     /* bridge base address */
    ldr r1, =0xdeadbeef
    str r1, [r0]            /* write to offset 0 (reg 0) */
    ldr r2, [r0]            /* read back from offset 0   */
loop:
    wfi
    b loop
EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$ASM_PATH" -o "$ELF_PATH"

# ── 4. Build minimal DTB ──────────────────────────────────────────────────────
echo "[phase5] Compiling device tree..."
cat > "$DTS_PATH" <<EOF
/dts-v1/;
/ {
    model = "virtmcu-phase5-test";
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

    bridge@50000000 {
        compatible = "mmio-socket-bridge";
        reg = <0x0 0x50000000 0x0 0x1000>;
        socket-path = "$SOCK_PATH";
        region-size = <0x1000>;
    };
};
EOF
dtc -I dts -O dtb -o "$DTB_PATH" "$DTS_PATH"

# ── 5. Start QEMU ─────────────────────────────────────────────────────────────
echo "[phase5] Starting QEMU..."
"$RUN_SH" --dtb "$DTB_PATH" \
    --kernel "$ELF_PATH" \
    -nographic \
    -monitor none \
    -qmp "unix:$QMP_SOCK,server,nowait" \
    > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

# ── 6. Wait for adapter to log the expected transactions ─────────────────────
echo "[phase5] Waiting for firmware transactions..."
PASSED=false
for _ in $(seq 1 50); do
    if grep -q "Wrote deadbeef to reg 0" "$ADAPTER_LOG" 2>/dev/null && \
       grep -q "Read deadbeef from reg 0"  "$ADAPTER_LOG" 2>/dev/null; then
        PASSED=true
        break
    fi
    # Check if QEMU or adapter died prematurely
    if ! kill -0 "$QEMU_PID"    2>/dev/null; then echo "[phase5] QEMU exited early"; break; fi
    if ! kill -0 "$ADAPTER_PID" 2>/dev/null; then echo "[phase5] Adapter exited early"; break; fi
    sleep 0.1
done

if [ "$PASSED" = true ]; then
    echo "[phase5] Phase 5 smoke test: PASSED"
    exit 0
else
    echo "[phase5] Phase 5 smoke test: FAILED"
    echo "--- Adapter log ---"
    cat "$ADAPTER_LOG"
    echo "--- QEMU log ---"
    cat "$QEMU_LOG"
    exit 1
fi
