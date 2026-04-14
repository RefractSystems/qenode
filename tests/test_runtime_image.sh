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

    echo "3. Checking yaml2qemu tool..."
    export PYTHONPATH="/opt/virtmcu:$PYTHONPATH"
    if ! python3 -m tools.yaml2qemu --help > /dev/null 2>&1; then
        echo "❌ Error: tools.yaml2qemu module not found"
        exit 1
    fi

    echo "4. Checking virtmcu plugins..."
    if ! qemu-system-arm -device help | grep -q "zenoh-clock"; then
        echo "❌ Error: virtmcu zenoh-clock device not found"
        exit 1
    fi
'
echo "✅ Runtime image verification passed!"
