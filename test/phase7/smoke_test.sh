#!/usr/bin/env bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Generate a dummy DTB to satisfy arm-generic-fdt requirement
cat <<'DTS_EOF' > /tmp/phase7_dummy.dts
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
DTS_EOF
dtc -I dts -O dtb -o /tmp/phase7_dummy.dtb /tmp/phase7_dummy.dts

cat <<'ASM_EOF' > /tmp/phase7_firmware.S
.global _start
_start:
loop:
    b loop
ASM_EOF
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -g -T "$WORKSPACE_DIR/test/phase1/linker.ld" /tmp/phase7_firmware.S -o /tmp/phase7_firmware.elf

cat << 'PY_EOF' > /tmp/test_phase7.py
import zenoh
import time
import struct
import sys

def main():
    session = zenoh.open(zenoh.Config())
    print("Session opened")
    
    delta_ns = 1000000 # 1ms
    mujoco_time = 0
    payload = struct.pack("<QQ", delta_ns, mujoco_time)
    
    print("Sending query 1...")
    replies = session.get("sim/clock/advance/0", payload=payload, timeout=2.0)
    for reply in replies:
        if hasattr(reply, "ok"):
            data = reply.ok.payload.to_bytes()
            vtime, _ = struct.unpack("<QI", data)
            print(f"Q1 OK: vtime = {vtime}")
        else:
            print("Q1 ERR")
            sys.exit(1)
            
    print("Sending query 2...")
    replies = session.get("sim/clock/advance/0", payload=payload, timeout=2.0)
    for reply in replies:
        if hasattr(reply, "ok"):
            data = reply.ok.payload.to_bytes()
            vtime, _ = struct.unpack("<QI", data)
            print(f"Q2 OK: vtime = {vtime}")
        else:
            print("Q2 ERR")
            sys.exit(1)

    session.close()

if __name__ == "__main__":
    main()
PY_EOF

echo "Starting QEMU with Zenoh Clock (suspend mode)..."
"$WORKSPACE_DIR/scripts/run.sh" --dtb /tmp/phase7_dummy.dtb \
    -kernel /tmp/phase7_firmware.elf \
    -device zenoh-clock,mode=suspend,node=0 \
    -nographic \
    -monitor none > /tmp/qemu_phase7_suspend.log 2>&1 &
QEMU_PID=$!

sleep 2
python3 /tmp/test_phase7.py
kill $QEMU_PID || true

echo "Starting QEMU with Zenoh Clock (icount mode)..."
"$WORKSPACE_DIR/scripts/run.sh" --dtb /tmp/phase7_dummy.dtb \
    -kernel /tmp/phase7_firmware.elf \
    -icount shift=0,align=off,sleep=off \
    -device zenoh-clock,mode=icount,node=0 \
    -nographic \
    -monitor none > /tmp/qemu_phase7_icount.log 2>&1 &
QEMU_PID=$!

sleep 2
python3 /tmp/test_phase7.py
kill $QEMU_PID || true

rm -f /tmp/phase7_* /tmp/test_phase7.py
