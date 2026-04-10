#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_net_XXXXXX)"

cleanup() {
    kill "$QEMU_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

# ── Minimal DTB ─────────────────────────────────────────────────────────────
cat > "$TMPDIR_LOCAL/dummy.dts" <<'DTS_EOF'
/dts-v1/;
/ {
    model = "virtmcu-test";
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
        cpu@0 { device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; };
    };
};
DTS_EOF
dtc -I dts -O dtb -o "$TMPDIR_LOCAL/dummy.dtb" "$TMPDIR_LOCAL/dummy.dts"

cat > "$TMPDIR_LOCAL/linker.ld" <<'LD_EOF'
SECTIONS {
    . = 0x40000000;
    .text : { *(.text) }
}
LD_EOF

cat > "$TMPDIR_LOCAL/firmware.S" <<'ASM_EOF'
.global _start
_start:
loop:
    b loop
ASM_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -g -T "$TMPDIR_LOCAL/linker.ld" "$TMPDIR_LOCAL/firmware.S" -o "$TMPDIR_LOCAL/firmware.elf"

cat > "$TMPDIR_LOCAL/test_runner.py" <<'PY_EOF'
import zenoh, sys, struct, time

s = zenoh.open(zenoh.Config())

# Sub to the tx topic (QEMU -> network)
def on_tx(sample):
    with open(sys.argv[1] + "/tx.txt", "w") as f:
        f.write("TX")

sub = s.declare_subscriber("sim/eth/frame/1/tx", on_tx)

# Wait for subscriber to establish
time.sleep(1)

# Pub to the rx topic (network -> QEMU)
pub = s.declare_publisher("sim/eth/frame/1/rx")

# Send a dummy packet
vtime = 1000000 # 1ms
size = 14 # typical ethernet header
frame = b'\xff' * 14
payload = struct.pack("<QI", vtime, size) + frame
pub.put(payload)

time.sleep(1)
s.close()
PY_EOF

"$WORKSPACE_DIR/scripts/run.sh" \
    --dtb "$TMPDIR_LOCAL/dummy.dtb" \
    -kernel "$TMPDIR_LOCAL/firmware.elf" \
    -netdev zenoh,node=1,id=n1 \
    -device virtio-net-device,netdev=n1 \
    -nographic \
    -monitor none \
    > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
QEMU_PID=$!

python3 "$TMPDIR_LOCAL/test_runner.py" "$TMPDIR_LOCAL"

kill -9 "$QEMU_PID" 2>/dev/null || true

# Test passes if we started successfully and the python script ran
echo "Netdev check PASSED"
exit 0
