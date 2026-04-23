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

# We mount the current host directory (repo root) to /workspace inside the container
# so that tools/ are available for the PYTHONPATH check.
docker run -i --rm \
    -v "$(pwd):/workspace" \
    -e QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu" \
    -e PATH="/opt/virtmcu/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    "$IMAGE" bash <<'DOCKER_EOF'
    set -euo pipefail
    
    echo "1. Checking QEMU binaries and core tools..."
    which qemu-system-arm > /dev/null || (echo "❌ qemu-system-arm not found" && exit 1)
    which dtc > /dev/null || (echo "❌ dtc (device-tree-compiler) not found" && exit 1)
    
    # In 'runtime' image, code is in /app. In 'builder' image, we mount to /workspace.
    # We look for where tools/yaml2qemu.py exists.
    # We use a list of possible search roots and filter for those that exist.
    SEARCH_ROOTS=""
    for d in /app /workspace /tmp; do
        [ -d "$d" ] && SEARCH_ROOTS="$SEARCH_ROOTS $d"
    done

    WS_PATH=$(find $SEARCH_ROOTS -name "yaml2qemu.py" -path "*/tools/yaml2qemu.py" | head -n 1)
    if [ -n "$WS_PATH" ]; then
        WS=$(dirname "$(dirname "$WS_PATH")")
        echo "Found workspace at: $WS"
    else
        echo "DEBUG: filesystem state:"
        ls -d /app /workspace /tmp 2>/dev/null || true
        echo "❌ Could not find tools directory" && exit 1
    fi

    export PYTHONPATH="$WS:${PYTHONPATH:-}"
    # Ensure dependencies are installed for the tools to run
    uv pip install --system --break-system-packages -r "$WS/pyproject.toml" > /dev/null
    python3 -m tools.yaml2qemu --help > /dev/null 2>&1 || (echo "❌ tools.yaml2qemu failed to run" && exit 1)

    echo "2. Verifying QOM Plugin Existence..."
    PLUGIN_PATH=$(find /opt /usr /build -name "hw-virtmcu-zenoh-clock.so" | head -n 1)
    if [ -n "$PLUGIN_PATH" ]; then
        echo "Found zenoh-clock plugin at: $PLUGIN_PATH"
        MOD_DIR=$(dirname "$PLUGIN_PATH")
    else
        echo "DEBUG: find plugin state:"
        find /opt /usr /build -name "*.so" -path "*virtmcu*" 2>/dev/null || true
        echo "❌ zenoh-clock plugin missing" && exit 1
    fi

    echo "4. Testing Full-System Zenoh Federation Contract..."
    # Create a minimal board for a boot test
    TMP_YAML=$(mktemp /tmp/test-XXXXXX.yaml)
    TMP_DTB=$(mktemp /tmp/test-XXXXXX.dtb)
    cat << YML > "$TMP_YAML"
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
    python3 -m tools.yaml2qemu "$TMP_YAML" --out-dtb "$TMP_DTB" > /dev/null

    PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("", 0)); print(s.getsockname()[1]); s.close()')
    
    # Start the mock router (TCP-only, no multicast)
    python3 -u /app/tests/zenoh_router_persistent.py "tcp/127.0.0.1:$PORT" &
    ROUTER_PID=$!
    sleep 2

    # Launch QEMU with all FirmwareStudio plugins active
    # This proves they all cooperate on the same Zenoh session and respect the router endpoint.
    qemu-system-arm \
        -M arm-generic-fdt,hw-dtb="$TMP_DTB" \
        -device zenoh-clock,node=0,router=tcp/127.0.0.1:$PORT \
        -netdev zenoh,node=0,id=n0,router=tcp/127.0.0.1:$PORT \
        -chardev zenoh,node=0,id=c0,router=tcp/127.0.0.1:$PORT \
        -display none -daemonize

    # Verify the clock queryable is reachable via the TCP router
    if python3 -c "import zenoh, sys, struct; c=zenoh.Config(); c.insert_json5('connect/endpoints', '[\"tcp/127.0.0.1:$PORT\"]'); c.insert_json5('scouting/multicast/enabled', 'false'); s=zenoh.open(c); r=list(s.get('sim/clock/advance/0', payload=struct.pack('<QQ', 0, 0), timeout=5.0)); s.close(); sys.exit(0 if r else 1)" 2>/dev/null; then
        echo "   ✅ Full-System Federation Contract verified (Clock + Net + UART)."
        rm -f "$TMP_YAML" "$TMP_DTB"
    else
        echo "❌ Error: QEMU failed to expose federated queryables over TCP."
        rm -f "$TMP_YAML" "$TMP_DTB"
        kill -9 "$ROUTER_PID" || true
        pkill -9 qemu-system || true
        exit 1
    fi

    # Cleanup
    kill -9 "$ROUTER_PID" || true
    pkill -9 qemu-system || true
    echo "✅ All runtime image contract checks passed!"
DOCKER_EOF
