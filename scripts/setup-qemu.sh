#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"

if [ ! -d "$QEMU_DIR/.git" ]; then
    echo "QEMU submodule not initialized. Please run git submodule update --init --recursive"
    exit 1
fi

cd "$QEMU_DIR"

# Ensure we are on the expected version
VERSION=$(cat VERSION || echo "")
if [[ "$VERSION" != *"10.2.92"* ]] && [[ "$VERSION" != *"11.0.0-rc2"* ]]; then
    echo "Unexpected QEMU version: $VERSION"
    exit 1
fi

# Apply patch series if not already applied
if ! git log | grep -q "arm-generic-fdt"; then
    echo "Applying arm-generic-fdt-v3 patch series..."
    git am --3way "$WORKSPACE_DIR/patches/arm-generic-fdt-v3.mbx"
else
    echo "arm-generic-fdt patch already applied."
fi

# Apply custom patches
cd "$WORKSPACE_DIR"
python3 patches/apply_libqemu.py third_party/qemu
python3 patches/apply_zenoh_hook.py third_party/qemu

# Symlink hw/ into QEMU's hw/qenode
ln -sfn "$WORKSPACE_DIR/hw" "$QEMU_DIR/hw/qenode"
if ! grep -q "subdir('qenode')" "$QEMU_DIR/hw/meson.build"; then
    echo "subdir('qenode')" >> "$QEMU_DIR/hw/meson.build"
fi

# Configure and build
cd "$QEMU_DIR"
mkdir -p build-qenode
cd build-qenode

# Handle macOS plugins bug (GitLab #516)
if [ "$(uname)" = "Darwin" ]; then
    echo "macOS detected: disabling --enable-plugins to avoid GLib module conflicts"
    ../configure --enable-modules --enable-fdt --enable-debug --target-list=arm-softmmu,arm-linux-user --prefix="$(pwd)/install"
else
    ../configure --enable-modules --enable-fdt --enable-plugins --enable-debug --target-list=arm-softmmu,arm-linux-user --prefix="$(pwd)/install"
fi

make -j$(nproc)
echo "QEMU build completed successfully."
