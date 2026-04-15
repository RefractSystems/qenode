#!/usr/bin/env bash
# test/phase7/determinism_test.sh — Phase 7 determinism test for slaved-icount mode.
#
# Verifies:
#   1. Boot a minimal firmware that increments a memory location in a tight loop.
#   2. Step the virtual clock exactly 1000 times by 1 ms (1 second total).
#   3. Query the memory location via QMP to observe the exact instruction boundary reached.
#   4. Repeat the entire process.
#   5. Assert that the memory values perfectly match across both runs, proving 
#      instruction-level determinism.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMPDIR_LOCAL="$(mktemp -d /tmp/phase7_det_XXXXXX)"

QEMU_PID=""
cleanup() {
    [[ -n "$QEMU_PID" ]] && kill -9 "$QEMU_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_LOCAL"
}
trap cleanup EXIT

# ── Firmware ────────────────────────────────────────────────────────────────
cat > "$TMPDIR_LOCAL/linker.ld" <<'LD_EOF'
SECTIONS {
    . = 0x40000000;
    .text : { *(.text) }
}
LD_EOF

cat > "$TMPDIR_LOCAL/firmware.S" <<'ASM_EOF'
.global _start
_start:
    ldr r1, =0x40001000
loop:
    /* Increment a counter in memory. If execution is deterministic, 
       the counter value at every quantum boundary will be identical across runs. */
    ldr r0, [r1]
    add r0, r0, #1
    str r0, [r1]
    b loop
ASM_EOF

arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -g -T "$TMPDIR_LOCAL/linker.ld" "$TMPDIR_LOCAL/firmware.S" -o "$TMPDIR_LOCAL/firmware.elf"

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

# ── Test Script ─────────────────────────────────────────────────────────────
cat > "$TMPDIR_LOCAL/test_runner.py" <<'PY_EOF'
import zenoh, sys, struct, socket, json, time

s = zenoh.open(zenoh.Config())
p = struct.pack("<QQ", 1000000, 0) # 1ms

# Advance clock 1000 times
for i in range(1000):
    r = list(s.get("sim/clock/advance/0", payload=p, timeout=2.0))
    if not r:
        print("Failed to get clock advance reply", file=sys.stderr)
        sys.exit(1)

s.close()

# Connect via QMP to read memory at 0x40001000
qmp = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
for _ in range(10):
    try:
        qmp.connect(sys.argv[1])
        break
    except:
        time.sleep(0.5)

qmp.recv(1024)
qmp.send(json.dumps({"execute": "qmp_capabilities"}).encode() + b"\n")
qmp.recv(1024)

qmp.send(json.dumps({"execute": "human-monitor-command", "arguments": {"command-line": "x /1xw 0x40001000"}}).encode() + b"\n")
resp = ""
while "\n" not in resp:
    resp += qmp.recv(4096).decode()

try:
    val = json.loads(resp)["return"].strip()
    print(val)
except:
    print(f"Error parsing QMP response: {resp}", file=sys.stderr)
    sys.exit(1)
PY_EOF

run_test() {
    "$WORKSPACE_DIR/scripts/run.sh" \
        --dtb "$TMPDIR_LOCAL/dummy.dtb" \
        -kernel "$TMPDIR_LOCAL/firmware.elf" \
        -icount shift=0,align=off,sleep=off \
        -device zenoh-clock,mode=icount,node=0,router=tcp/127.0.0.1:7447 \
        -nographic \
        -monitor none \
        -qmp "unix:$TMPDIR_LOCAL/qmp.sock,server,nowait" \
        > "$TMPDIR_LOCAL/qemu.log" 2>&1 &
    QEMU_PID=$!

    # Wait for queryable
    deadline=$(( $(date +%s) + 15 ))
    ready=0
    while (( $(date +%s) < deadline )); do
        if python3 -c 'import zenoh, sys, struct; s=zenoh.open(zenoh.Config()); r=list(s.get("sim/clock/advance/0", payload=struct.pack("<QQ",0,0), timeout=0.5)); s.close(); sys.exit(0 if r else 1)' 2>/dev/null; then
            ready=1
            break
        fi
        sleep 0.25
    done
    if [ $ready -eq 0 ]; then
        echo "Failed to find queryable"
        exit 1
    fi

    val=$(python3 "$TMPDIR_LOCAL/test_runner.py" "$TMPDIR_LOCAL/qmp.sock")
    kill -9 "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true
    rm -f "$TMPDIR_LOCAL/qmp.sock"
    echo "$val"
}

echo "Running determinism test Run 1..."
VAL1=$(run_test)
echo "Run 1 read: $VAL1"

echo "Running determinism test Run 2..."
VAL2=$(run_test)
echo "Run 2 read: $VAL2"

if [ -z "$VAL1" ] || [ -z "$VAL2" ]; then
    echo "Determinism check FAILED: Empty response"
    exit 1
fi

if [ "$VAL1" = "$VAL2" ]; then
    echo "Determinism check PASSED: Memory values match exactly."
    exit 0
else
    echo "Determinism check FAILED: $VAL1 != $VAL2"
    exit 1
fi
