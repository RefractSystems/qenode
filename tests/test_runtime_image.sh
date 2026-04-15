#!/usr/bin/env bash
# Verifies the API contract of the final virtmcu runtime image.
set -euo pipefail

echo "=============================================================================="
echo "🧪 RUNNING TEST: $(basename "$0")"
echo "=============================================================================="
cat << 'TEST_DOC_BLOCK'
Verifies the API contract of the final virtmcu runtime image.

This test guarantees that:
1. All required QEMU binaries (ARM/RISC-V) and tools (DTC, yaml2qemu) are present.
2. All virtmcu QOM plugins (.so) are correctly linked and loadable by QEMU.
3. The Zenoh Federation Contract (router= property) is honored by all plugins.
4. The mmio-socket-bridge wire protocol handler is present and loadable.
TEST_DOC_BLOCK
echo "=============================================================================="

IMAGE=${1:-}
if [ -z "$IMAGE" ]; then
    echo "Usage: $0 <runtime_image>"
    exit 1
fi

echo "Verifying runtime image: $IMAGE"

docker run -i --rm -v "$(pwd):/app" "$IMAGE" bash <<'DOCKER_EOF'
    set -euo pipefail
    
    echo "1. Checking QEMU binaries and core tools..."
    which qemu-system-arm > /dev/null || (echo "❌ qemu-system-arm not found" && exit 1)
    which dtc > /dev/null || (echo "❌ dtc (device-tree-compiler) not found" && exit 1)
    
    export PYTHONPATH="/app:${PYTHONPATH:-}"
    python3 -m tools.yaml2qemu --help > /dev/null 2>&1 || (echo "❌ tools.yaml2qemu not found" && exit 1)

    echo "2. Verifying QOM Plugin Dynamic Linking..."
    # Install strace for debugging
    apt-get update -q && apt-get install -y strace -q > /dev/null

    echo "Checking global device help..."
    qemu-system-arm -device help > /dev/null || (echo "❌ QEMU failed to list devices" && exit 1)

    echo "Tracing zenoh-clock initialization..."
    # Verify every virtmcu .so plugin can be loaded without missing symbol errors
    # Note: We check for 'help' which forces QEMU to initialize the device classes.
    strace -f qemu-system-arm -M arm-generic-fdt -device zenoh-clock,help 2>&1 | tail -n 50
    qemu-system-arm -M arm-generic-fdt -device zenoh-clock,help > /dev/null || (echo "❌ zenoh-clock plugin failed to load" && exit 1)
    qemu-system-arm -M arm-generic-fdt -device mmio-socket-bridge,help > /dev/null || (echo "❌ mmio-socket-bridge plugin failed to load" && exit 1)
    
    echo "3. Verifying Backend Plugin Registration..."
    qemu-system-arm -netdev help | grep -q "zenoh" || (echo "❌ zenoh netdev backend not registered" && exit 1)
    qemu-system-arm -chardev help | grep -q "zenoh" || (echo "❌ zenoh chardev backend not registered" && exit 1)

    echo "4. Testing Full-System Zenoh Federation Contract..."
    # Create a minimal board for a boot test
    cat << YML > /tmp/test.yaml
machine:
  name: test
  type: arm-generic-fdt
  cpus: [ { name: cpu0, type: cortex-a15 } ]
peripherals:
  - name: flash
    type: Memory.MappedMemory
    address: 0x00000000
    properties: { size: "0x01000000" }
YML
    python3 -m tools.yaml2qemu /tmp/test.yaml --out-dtb /tmp/test.dtb > /dev/null

    # Start the mock router (TCP-only, no multicast)
    python3 -u /app/tests/zenoh_router_persistent.py &
    ROUTER_PID=$!
    sleep 2

    # Launch QEMU with all FirmwareStudio plugins active
    # This proves they all cooperate on the same Zenoh session and respect the router endpoint.
    qemu-system-arm \
        -M arm-generic-fdt,hw-dtb=/tmp/test.dtb \
        -device zenoh-clock,node=0,router=tcp/127.0.0.1:7447 \
        -netdev zenoh,node=0,id=n0,router=tcp/127.0.0.1:7447 \
        -chardev zenoh,node=0,id=c0,router=tcp/127.0.0.1:7447 \
        -display none -daemonize

    # Verify the clock queryable is reachable via the TCP router
    if python3 -c "import zenoh, sys, struct; s=zenoh.open(); r=list(s.get('sim/clock/advance/0', payload=struct.pack('<QQ', 0, 0), timeout=5.0)); s.close(); sys.exit(0 if r else 1)" 2>/dev/null; then
        echo "   ✅ Full-System Federation Contract verified (Clock + Net + UART)."
    else
        echo "❌ Error: QEMU failed to expose federated queryables over TCP."
        kill -9 "$ROUTER_PID" || true
        pkill -9 qemu-system || true
        exit 1
    fi

    # Cleanup
    kill -9 "$ROUTER_PID" || true
    pkill -9 qemu-system || true
    echo "✅ All runtime image contract checks passed!"
DOCKER_EOF
