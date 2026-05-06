#!/usr/bin/env bash
set -euo pipefail

# Helper for sudo if not root
SUDO=""
if [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
fi

$SUDO uv pip install --link-mode=copy --system --break-system-packages pyelftools >/dev/null 2>&1

make -C tests/fixtures/guest_apps/boot_arm

# Find drcov plugin (priority: installed prefix, then build tree)
DRCOV_SO=$(find /build/qemu third_party/qemu/build-virtmcu -name "libdrcov.so" 2>/dev/null | head -n 1)
if [ -z "$DRCOV_SO" ]; then
    echo "Error: libdrcov.so not found in third_party or /build/qemu"
    exit 1
fi
echo "Using drcov plugin: $DRCOV_SO"

# Run with drcov plugin and kill with SIGINT after 2s to flush data
qemu-system-arm -M arm-generic-fdt,hw-dtb=tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -kernel tests/fixtures/guest_apps/boot_arm/hello.elf -nographic -m 128M -display none \
    -plugin "$DRCOV_SO",filename=hello.drcov \
    -d plugin &

sleep 2
kill -INT $!
wait $! || true

# Analyze results
python3 tools/analyze_coverage.py hello.drcov tests/fixtures/guest_apps/boot_arm/hello.elf --fail-under 80
