#!/usr/bin/env bash
# ==============================================================================
# setup-qemu.sh
#
# This script initializes, patches, configures, and builds the QEMU emulator
# used by the virtmcu project. It performs the following steps:
#   1. Clones QEMU (--depth=1) into third_party/qemu if not already present.
#   2. Applies the 'arm-generic-fdt' patch series via `git am`.
#   3. Applies custom AST-injection patches (libqemu and zenoh hooks) to QEMU C code.
#   4. Symlinks the project's custom `hw/` directory into QEMU's build tree.
#   5. Configures QEMU (handling macOS specific flags if necessary).
#   6. Compiles and installs the QEMU binaries to `third_party/qemu/build-virtmcu/install`.
# ==============================================================================

set -e

# Determine absolute paths for the script, workspace, and QEMU directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"

# Clone QEMU if not already present
QEMU_REPO="${QEMU_REPO:-https://gitlab.com/qemu-project/qemu.git}"
QEMU_REF="${QEMU_REF:-v11.0.0-rc3}"

if [ ! -d "$QEMU_DIR/.git" ]; then
    echo "==> Cloning QEMU ${QEMU_REF} from ${QEMU_REPO} ..."
    mkdir -p "$WORKSPACE_DIR/third_party"
    git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" "$QEMU_DIR"
    cd "$QEMU_DIR"
    git submodule update --init --recursive --depth=1
    cd "$QEMU_DIR"
    git config user.email "virtmcu-build@example.com"
    git config user.name "virtmcu"
fi

cd "$QEMU_DIR"

# Ensure we are on the expected QEMU version (11.0.0-rc3)
VERSION=$(cat VERSION || echo "")
if [[ "$VERSION" != *"10.2.9"* ]] && [[ "$VERSION" != *"11.0.0-rc"* ]]; then
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

# Phase 2: Allow dynamic loading of SysBus devices via `-device`
# The arm-generic-fdt patch does not set this by default, which breaks out-of-tree plugins.
if ! grep -q "machine_class_allow_dynamic_sysbus_dev(mc, \"sys-bus-device\")" "$QEMU_DIR/hw/arm/arm_generic_fdt.c"; then
    echo "Enabling dynamic sysbus devices for arm-generic-fdt..."
    sed -i 's/mc->minimum_page_bits = 12;/mc->minimum_page_bits = 12;\n\n    \/* virtmcu: allow all SysBus devices via -device; arm-generic-fdt loads devices from DTB at runtime *\/\n    machine_class_allow_dynamic_sysbus_dev(mc, "sys-bus-device");/' "$QEMU_DIR/hw/arm/arm_generic_fdt.c"
fi

# Symlink our custom hw/ directory into QEMU's hw/virtmcu directory
# This allows QEMU's Meson build system to compile our custom peripherals
ln -sfn "$WORKSPACE_DIR/hw" "$QEMU_DIR/hw/virtmcu"
# Inject 'subdir('virtmcu')' into QEMU's hw/meson.build if not already there
if ! grep -q "subdir('virtmcu')" "$QEMU_DIR/hw/meson.build"; then
    echo "subdir('virtmcu')" >> "$QEMU_DIR/hw/meson.build"
fi

# Configure and build QEMU in a dedicated build directory
cd "$QEMU_DIR"
mkdir -p build-virtmcu
cd build-virtmcu

# Configure the build, handling macOS specific plugin bugs (GitLab #516)
if [ "$(uname)" = "Darwin" ]; then
    echo "macOS detected: disabling --enable-plugins to avoid GLib module conflicts"
    ../configure --enable-modules --enable-fdt --enable-debug --target-list=arm-softmmu,arm-linux-user --prefix="$(pwd)/install"
else
    ../configure --enable-modules --enable-fdt --enable-plugins --enable-debug --target-list=arm-softmmu,arm-linux-user --prefix="$(pwd)/install"
fi

# Compile QEMU using all available CPU cores
make -j$(nproc)
# Install QEMU binaries to the prefix directory (build-virtmcu/install)
make install
echo "QEMU build and install completed successfully."
