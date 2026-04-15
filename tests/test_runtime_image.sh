#!/usr/bin/env bash
# Verifies the API contract of the final virtmcu runtime image.
set -e

IMAGE=$1
if [ -z "$IMAGE" ]; then
    echo "Usage: $0 <runtime_image>"
    exit 1
fi

echo "Verifying runtime image: $IMAGE"

docker run -i --rm -v "$(pwd):/app" "$IMAGE" bash <<'EOF'
    set -e
    echo "1. Checking QEMU binary..."
    which qemu-system-arm > /dev/null || (echo "❌ qemu-system-arm not found" && exit 1)
    
    echo "2. Checking arm-generic-fdt support..."
    qemu-system-arm -M help | grep -q "arm-generic-fdt" || (echo "❌ arm-generic-fdt not supported" && exit 1)

    echo "3. Checking yaml2qemu tool and dtc..."
    which dtc > /dev/null || (echo "❌ dtc not found" && exit 1)
    export PYTHONPATH="/app:$PYTHONPATH"
    python3 -m tools.yaml2qemu --help > /dev/null 2>&1 || (echo "❌ tools.yaml2qemu not found" && exit 1)

    echo "4. Checking virtmcu plugins..."
    if ! find /opt/virtmcu/lib -name "hw-virtmcu-zenoh.so" | grep -q "."; then
        echo "❌ Error: hw-virtmcu-zenoh.so plugin not found"
        exit 1
    fi

    echo "4b. Checking mmio-socket-bridge instantiation..."
    # Verify the plugin actually loads into QEMU without missing symbols or segfaults
    qemu-system-arm -M arm-generic-fdt -device mmio-socket-bridge,help > /dev/null || (echo "❌ mmio-socket-bridge failed to initialize" && exit 1)

    echo "5. Checking Zenoh connectivity (router= TCP property)..."

    # Use the pre-built phase1 firmware and DTB (checked in; no toolchain needed).
    # arm-generic-fdt machine supports -device zenoh-clock via the dynamic-sysbus
    # patch applied in the Dockerfile.
    PHASE1_DTB="/app/test/phase1/minimal.dtb"
    PHASE1_ELF="/app/test/phase1/hello.elf"
    if [ ! -f "$PHASE1_DTB" ] || [ ! -f "$PHASE1_ELF" ]; then
        echo "❌ Phase1 test artifacts not found at $PHASE1_DTB / $PHASE1_ELF"
        exit 1
    fi

    # Start the mock router first so port 7447 is ready before QEMU connects.
    # The mock listens on TCP with multicast disabled — if QEMU ignores router=
    # and uses multicast instead, the GET never reaches it and the test fails.
    export PYTHONPATH="/app:$PYTHONPATH"
    python3 -u /app/tests/zenoh_router_mock.py &
    ROUTER_PID=$!

    sleep 2

    # QEMU must run (not -S): the clock-advance handshake requires the vCPU hook
    # to fire at a TB boundary so on_query can complete and send its reply.
    qemu-system-arm \
        -M arm-generic-fdt,hw-dtb="$PHASE1_DTB" \
        -kernel "$PHASE1_ELF" \
        -device zenoh-clock,router=tcp/127.0.0.1:7447,node=0 \
        -nographic \
        -monitor none \
        > /tmp/qemu_zenoh.log 2>&1 &
    QEMU_PID=$!

    if wait "$ROUTER_PID"; then
        echo "✅ Zenoh TCP router connectivity verified"
    else
        echo "❌ Zenoh TCP router connectivity test failed"
        echo "--- QEMU LOG ---"
        cat /tmp/qemu_zenoh.log
        echo "----------------"
        kill "$QEMU_PID" 2>/dev/null || true
        exit 1
    fi
    kill "$QEMU_PID" 2>/dev/null || true
EOF

echo "✅ Runtime image verification passed!"
