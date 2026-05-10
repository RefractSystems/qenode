#!/usr/bin/env bash
set -euo pipefail

# Helper for sudo if not root
SUDO=""
if [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
fi

$SUDO uv pip install --link-mode=copy --system --break-system-packages pyelftools >/dev/null 2>&1

make -C tests/fixtures/guest_apps/boot_arm

# Find drcov plugin
SEARCH_PATHS=("/usr/local/lib/qemu/plugins" "/build/qemu" "third_party/qemu/build-virtmcu")
DRCOV_SO=""
for p in "${SEARCH_PATHS[@]}"; do
    if [ -d "$p" ]; then
        FOUND=$(find "$p" -name "libdrcov.so" 2>/dev/null | head -n 1)
        if [ -n "$FOUND" ]; then
            DRCOV_SO="$FOUND"
            break
        fi
    fi
done

if [ -z "$DRCOV_SO" ]; then
    echo "❌ Error: libdrcov.so not found"
    exit 1
fi
echo "==> Using drcov plugin: $DRCOV_SO"

mkdir -p coverage-data
cargo build --release -p virtmcu-run

# Run with drcov plugin and kill with SIGINT after 2s to flush data
(
    ./target/release/virtmcu-run --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb \
        -kernel tests/fixtures/guest_apps/boot_arm/hello.elf -nographic -m 128M -display none \
        -plugin "$DRCOV_SO",filename=coverage-data/hello.drcov \
        -d plugin > qemu.log 2>&1 &
    QEMU_PID=$!
    sleep 2
    kill -INT $QEMU_PID
    # Wait for up to 5 seconds for QEMU to exit
    for _ in {1..50}; do
        if ! kill -0 $QEMU_PID 2>/dev/null; then
            break
        fi
        sleep 0.1
    done

    # Force kill if still alive
    kill -9 $QEMU_PID 2>/dev/null || true
)

# Check if drcov file was created and is not empty
if [ ! -s coverage-data/hello.drcov ]; then
    echo "❌ Error: hello.drcov is empty or was not created. QEMU log:"
    cat qemu.log
    exit 1
fi

# Analyze results
python3 tools/analyze_coverage.py coverage-data/hello.drcov tests/fixtures/guest_apps/boot_arm/hello.elf --fail-under 80
