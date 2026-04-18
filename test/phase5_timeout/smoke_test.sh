#!/usr/bin/env bash
# ==============================================================================
# timeout_test.sh (Phase 5.6 — mmio-socket-bridge timeout)
#
# Verifies that QEMU does not hang when the MMIO socket bridge target is
# unresponsive or crashes.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

# Test artifacts
SOCK_PATH="/tmp/virtmcu-timeout-$$.sock"
QMP_SOCK="/tmp/qmp-timeout-$$.sock"
ADAPTER_LOG="/tmp/virtmcu-malicious-$$.log"
QEMU_LOG="/tmp/virtmcu-qemu-timeout-$$.log"
DTB_PATH="/tmp/virtmcu-timeout-$$.dtb"
DTS_PATH="/tmp/virtmcu-timeout-$$.dts"
ELF_PATH="/tmp/virtmcu-timeout-$$.elf"
ASM_PATH="/tmp/virtmcu-timeout-$$.S"
LD_PATH="/tmp/virtmcu-timeout-$$.ld"

cleanup() {
    kill "${QEMU_PID:-}"    2>/dev/null || true
    kill "${ADAPTER_PID:-}" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$QMP_SOCK" "$ADAPTER_LOG" "$QEMU_LOG" \
          "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$ASM_PATH" "$LD_PATH"
}
trap cleanup EXIT

# ── 1. Build common artifacts ────────────────────────────────────────────────
echo "[phase5.6] Building firmware and DTB..."

cat > "$LD_PATH" <<'EOF'
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text*) }
}
EOF

cat > "$ASM_PATH" <<'EOF'
.global _start
_start:
    ldr r0, =0x50000000
    ldr r1, [r0]            /* This read should trigger timeout or error */
loop:
    b loop
EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" "$ASM_PATH" -o "$ELF_PATH"

cat > "$DTS_PATH" <<EOF
/dts-v1/;
/ {
    model = "virtmcu-timeout-test";
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

run_test_case() {
    local mode=$1
    local expected_msg=$2

    echo "[phase5.6] RUNNING TEST CASE: $mode"
    
    # ── Start malicious adapter ──
    rm -f "$SOCK_PATH"
    python3 -u "$SCRIPT_DIR/malicious_adapter.py" "$SOCK_PATH" "$mode" > "$ADAPTER_LOG" 2>&1 &
    ADAPTER_PID=$!

    for _ in $(seq 1 50); do
        [ -S "$SOCK_PATH" ] && break
        sleep 0.1
    done

    if [ ! -S "$SOCK_PATH" ]; then
        echo "[phase5.6] FAILED: Adapter failed to start"
        cat "$ADAPTER_LOG"
        return 1
    fi

    # ── Start QEMU ──
    rm -f "$QEMU_LOG" "$QMP_SOCK"
    export QEMU_MODULE_DIR="$WORKSPACE_DIR/third_party/qemu/build-virtmcu"
    "$RUN_SH" --dtb "$DTB_PATH" \
        --kernel "$ELF_PATH" \
        -nographic \
        -monitor none \
        -d guest_errors \
        -qmp "unix:$QMP_SOCK,server,nowait" > "$QEMU_LOG" 2>&1 &
    QEMU_PID=$!

    # Wait for QEMU to start and encounter the MMIO operation
    sleep 5

    # ── Check QMP responsiveness ──
    echo "[phase5.6]   Checking QMP responsiveness..."
    if ! python3 -c '
import socket, sys, json
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(2.0)
try:
    s.connect(sys.argv[1])
    s.recv(4096)
    s.sendall(b"{\"execute\": \"qmp_capabilities\"}\n")
    s.recv(4096)
    s.sendall(b"{\"execute\": \"query-status\"}\n")
    data = s.recv(4096).decode("utf-8")
    if "return" not in data: sys.exit(1)
except Exception as e:
    # print(e)
    sys.exit(1)
sys.exit(0)
' "$QMP_SOCK"; then
        echo "[phase5.6]   FAILED: QEMU is unresponsive (likely hung)"
        kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
        echo "--- QEMU LOG ---"
        cat "$QEMU_LOG"
        return 1
    fi

    # ── Check for expected error in QEMU log ──
    echo "[phase5.6]   Checking for expected error in log..."
    # Kill QEMU first to ensure logs are flushed
    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true
    
    if grep -q "$expected_msg" "$QEMU_LOG"; then
        echo "[phase5.6]   SUCCESS: Expected error detected"
    else
        echo "[phase5.6]   FAILED: Expected error not found in log"
        echo "--- QEMU LOG ---"
        cat "$QEMU_LOG"
        echo "--- ADAPTER LOG ---"
        cat "$ADAPTER_LOG"
        return 1
    fi

    kill "$ADAPTER_PID" 2>/dev/null || true
    wait "$ADAPTER_PID" 2>/dev/null || true
    return 0
}

# TEST 1: HANG (Timeout)
# Expected message from mmio-socket-bridge.c
if ! run_test_case "hang" "mmio-socket-bridge: timeout on socket fd"; then
    exit 1
fi

echo "----------------------------------------------------------------------"

# TEST 2: CRASH (Immediate disconnect)
if ! run_test_case "crash" "mmio-socket-bridge: remote disconnected"; then
    exit 1
fi

echo "[phase5.6] All tests passed!"
exit 0
