#!/usr/bin/env bash
# Verifies the API contract of the final virtmcu runtime image.
set -e

IMAGE=$1
if [ -z "$IMAGE" ]; then
    echo "Usage: $0 <runtime_image>"
    exit 1
fi

echo "Verifying runtime image: $IMAGE"

docker run --rm "$IMAGE" sh -c '
    set -e
    echo "1. Checking QEMU binary..."
    if ! which qemu-system-arm > /dev/null; then
        echo "❌ Error: qemu-system-arm not found on PATH"
        exit 1
    fi
    qemu-system-arm --version > /dev/null

    echo "2. Checking arm-generic-fdt support..."
    if ! qemu-system-arm -M help | grep -q "arm-generic-fdt"; then
        echo "❌ Error: arm-generic-fdt machine type not supported"
        exit 1
    fi

    echo "3. Checking yaml2qemu tool and dtc..."
    if ! which dtc > /dev/null; then
        echo "❌ Error: dtc (device-tree-compiler) not found"
        exit 1
    fi
    export PYTHONPATH="/opt/virtmcu:$PYTHONPATH"
    if ! python3 -m tools.yaml2qemu --help > /dev/null 2>&1; then
        echo "❌ Error: tools.yaml2qemu module not found"
        exit 1
    fi

    echo "4. Checking virtmcu plugins..."
    if [ ! -f "/opt/virtmcu/lib/qemu/hw-virtmcu-zenoh.so" ]; then
        echo "❌ Error: hw-virtmcu-zenoh.so plugin not found"
        exit 1
    fi
    if [ ! -f "/opt/virtmcu/lib/qemu/hw-virtmcu-mmio-socket-bridge.so" ]; then
        echo "❌ Error: hw-virtmcu-mmio-socket-bridge.so plugin not found"
        exit 1
    fi
    
    echo "5. Testing Zenoh Federation Contract (router property)..."
    cat << YML > /tmp/test.yaml
machine:
  name: test
  type: arm-generic-fdt
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: flash
    type: Memory.MappedMemory
    address: 0x00000000
    properties:
      size: "0x01000000"
YML
    python3 -m tools.yaml2qemu /tmp/test.yaml --out-dtb /tmp/test.dtb > /dev/null

    # Start zenohd router in the background (we need a local router for the queryable test)
    # The runtime image has no zenohd, so we just run the query locally.
    
    qemu-system-arm -M arm-generic-fdt,hw-dtb=/tmp/test.dtb \
        -device zenoh-clock,node=0,router=tcp/127.0.0.1:7447 \
        -display none -daemonize

    if python3 -c "
import zenoh, time
try:
    session = zenoh.open(zenoh.Config())
    replies = session.get("sim/clock/advance/0", timeout=5.0)
    found = False
    for r in replies:
        found = True
        break
    if not found:
        exit(1)
    exit(0)
except:
    exit(1)
"; then
        echo "   ✅ QEMU successfully exposed queryable via router property."
    else
        echo "❌ Error: QEMU failed to expose clock queryable on the network."
        pkill -9 qemu-system || true
        exit 1
    fi

    pkill -9 qemu-system || true
    echo "✅ All runtime checks passed!"
'
