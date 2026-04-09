#!/usr/bin/env bash
# ==============================================================================
# setup-qemu.sh
#
# This script initializes, patches, configures, and builds the QEMU emulator
# used by the qenode project. It performs the following steps:
#   1. Verifies the QEMU submodule is initialized and at the correct version.
#   2. Applies the 'arm-generic-fdt' patch series via `git am`.
#   3. Applies custom AST-injection patches (libqemu and zenoh hooks) to QEMU C code.
#   4. Symlinks the project's custom `hw/` directory into QEMU's build tree.
#   5. Configures QEMU (handling macOS specific flags if necessary).
#   6. Compiles and installs the QEMU binaries to `third_party/qemu/build-qenode/install`.
# ==============================================================================

set -e

# Determine absolute paths for the script, workspace, and QEMU directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"

# Check if QEMU submodule has been cloned
if [ ! -d "$QEMU_DIR/.git" ]; then
    echo "QEMU submodule not initialized. Please run git submodule update --init --recursive"
    exit 1
fi

cd "$QEMU_DIR"

# Ensure we are on the expected QEMU version (10.2.92 or 11.0.0-rc2)
VERSION=$(cat VERSION || echo "")
if [[ "$VERSION" != *"10.2.92"* ]] && [[ "$VERSION" != *"11.0.0-rc2"* ]]; then
    echo "Unexpected QEMU version: $VERSION"
    exit 1
fi

# Apply the arm-generic-fdt patch series if it hasn't been applied yet
# This enables the dynamic FDT-based machine initialization
if ! git log | grep -q "arm-generic-fdt"; then
    echo "Applying arm-generic-fdt-v3 patch series..."
    git am --3way "$WORKSPACE_DIR/patches/arm-generic-fdt-v3.mbx"
else
    echo "arm-generic-fdt patch already applied."
fi

# Apply custom Python-based AST-injection patches
cd "$WORKSPACE_DIR"
python3 patches/apply_libqemu.py third_party/qemu
python3 patches/apply_zenoh_hook.py third_party/qemu

# Symlink our custom hw/ directory into QEMU's hw/qenode directory
# This allows QEMU's Meson build system to compile our custom peripherals
ln -sfn "$WORKSPACE_DIR/hw" "$QEMU_DIR/hw/qenode"
# Inject 'subdir('qenode')' into QEMU's hw/meson.build if not already there
if ! grep -q "subdir('qenode')" "$QEMU_DIR/hw/meson.build"; then
    echo "subdir('qenode')" >> "$QEMU_DIR/hw/meson.build"
fi

# Configure and build QEMU in a dedicated build directory
cd "$QEMU_DIR"
mkdir -p build-qenode
cd build-qenode

# Configure the build, handling macOS specific plugin bugs (GitLab #516)
if [ "$(uname)" = "Darwin" ]; then
    echo "macOS detected: disabling --enable-plugins to avoid GLib module conflicts"
    ../configure --enable-modules --enable-fdt --enable-debug --target-list=arm-softmmu,arm-linux-user --prefix="$(pwd)/install"
else
    ../configure --enable-modules --enable-fdt --enable-plugins --enable-debug --target-list=arm-softmmu,arm-linux-user --prefix="$(pwd)/install"
fi

# Compile QEMU using all available CPU cores
make -j$(nproc)
# Install QEMU binaries to the prefix directory (build-qenode/install)
make install
echo "QEMU build and install completed successfully."
