#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUN_SH="$WORKSPACE_DIR/scripts/run.sh"

SOCK_PATH="/tmp/virtmcu-reconnect-$$.sock"
DTB_PATH="/tmp/virtmcu-reconnect-$$.dtb"
DTS_PATH="/tmp/virtmcu-reconnect-$$.dts"
ELF_PATH="/tmp/virtmcu-reconnect-$$.elf"
LD_PATH="/tmp/virtmcu-reconnect-$$.ld"
QEMU_LOG="/tmp/virtmcu-reconnect-qemu-$$.log"

cleanup() {
    kill "${QEMU_PID:-}"    2>/dev/null || true
    kill "${ADAPTER_PID:-}" 2>/dev/null || true
    rm -f "$SOCK_PATH" "$DTB_PATH" "$DTS_PATH" "$ELF_PATH" "$LD_PATH"
}
trap cleanup EXIT

echo "[reconnect] Building firmware..."
cat > "$LD_PATH" <<'EOF'
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text*) }
}
EOF
cat > /tmp/reconnect.S <<'EOF'
.global _start
_start:
    ldr r0, =0x50000000
loop:
    ldr r1, [r0]            /* This will fail initially, then succeed after reconnect */
    cmp r1, #0x42
    bne loop
    /* Success */
    ldr r3, =0x09000000
    mov r4, #'O'
    str r4, [r3]
    mov r4, #'K'
    str r4, [r3]
    mov r4, #'\n'
    str r4, [r3]
end:
    b end
EOF
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T "$LD_PATH" /tmp/reconnect.S -o "$ELF_PATH"

echo "[reconnect] Compiling device tree..."
cat > "$DTS_PATH" <<EOF
/dts-v1/;
/ {
    model = "virtmcu-reconnect-test";
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
        reg = <0x0 0x50000000 0x0 0x1000>;
        socket-path = "$SOCK_PATH";
        region-size = <0x1000>;
        reconnect-ms = <500>;
    };
};
EOF
dtc -I dts -O dtb -o "$DTB_PATH" "$DTS_PATH"

echo "[reconnect] Starting QEMU (adapter not yet started)..."
"$RUN_SH" --dtb "$DTB_PATH" --kernel "$ELF_PATH" -nographic -monitor none > "$QEMU_LOG" 2>&1 &
QEMU_PID=$!

sleep 2
echo "[reconnect] Starting stress adapter (acting as normal adapter)..."
g++ -O3 "$SCRIPT_DIR/stress_adapter.cpp" -o "$SCRIPT_DIR/reconnect_adapter"
# Modify adapter to return 0x42 on MMIO
# (My stress_adapter echoes back data, so I'll just use a python mock instead)

cat > /tmp/mock_adapter.py <<EOF
import os, socket, struct, time
VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1
SYSC_MSG_RESP = 0
SYSC_MSG_IRQ_SET = 1
SYSC_MSG_IRQ_CLEAR = 2

def run():
    if os.path.exists("$SOCK_PATH"): os.remove("$SOCK_PATH")
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind("$SOCK_PATH")
    s.listen(1)
    print("Mock adapter listening...")
    conn, addr = s.accept()
    print("Connected!")
    # Handshake
    hs = conn.recv(8)
    conn.sendall(hs)
    
    # Trigger IRQ
    print("Sending IRQ SET...")
    conn.sendall(struct.pack("<IIQ", SYSC_MSG_IRQ_SET, 0, 0))
    time.sleep(0.1)
    print("Sending IRQ CLEAR...")
    conn.sendall(struct.pack("<IIQ", SYSC_MSG_IRQ_CLEAR, 0, 0))

    while True:
        req = conn.recv(32)
        if not req: break
        # Send back 0x42
        resp = struct.pack("<IIQ", SYSC_MSG_RESP, 0, 0x42)
        conn.sendall(resp)
run()
EOF

python3 /tmp/mock_adapter.py &
ADAPTER_PID=$!

echo "[reconnect] Waiting for OK from QEMU..."
PASSED=false
for _ in $(seq 1 100); do
    if grep -q "OK" "$QEMU_LOG"; then
        PASSED=true
        break
    fi
    sleep 0.1
done

if [ "$PASSED" = true ]; then
    echo "[reconnect] PASSED"
    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true
else
    echo "[reconnect] FAILED"
    echo "--- QEMU LOG ---"
    cat "$QEMU_LOG"
    exit 1
fi
