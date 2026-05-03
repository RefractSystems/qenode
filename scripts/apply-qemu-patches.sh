#!/usr/bin/env bash
# ==============================================================================
# apply-qemu-patches.sh
#
# Centralized script to apply all virtmcu patches to a QEMU source tree.
# Used by both the Dockerfile (for CI/Containers) and setup-qemu.sh (for bare-metal dev).
#
# Usage:
#   scripts/apply-qemu-patches.sh <path_to_qemu_src>
# ==============================================================================

set -euo pipefail

QEMU_DIR="${1:-}"
if [ -z "$QEMU_DIR" ] || [ ! -d "$QEMU_DIR" ]; then
    echo "Usage: $0 <path_to_qemu_src>"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

if [ -f "$WORKSPACE_DIR/BUILD_DEPS" ]; then
    # shellcheck source=../BUILD_DEPS
    source "$WORKSPACE_DIR/BUILD_DEPS"
fi

echo "==> Applying virtmcu patches to QEMU at $QEMU_DIR"

cd "$QEMU_DIR"

if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    # Git requires an identity to apply patches via 'am' or 'apply'
    git config user.email "virtmcu-build@example.com"
    git config user.name "virtmcu"
fi

# 1. Apply the arm-generic-fdt patch series
apply_patch_series() {
    local mbx="$1"

    if git am --3way "$mbx" 2>&1; then
        return 0
    fi

    local attempt
    for attempt in $(seq 1 10); do
        if ! git status 2>/dev/null | grep "am session" >/dev/null; then
            return 0
        fi
        echo "  [am retry $attempt] applying current patch with git apply..."
        if git am --show-current-patch=diff | git apply --3way; then
            git add -A
            git am --continue 2>&1 || true
        else
            echo "ERROR: git apply also failed — manual intervention needed."
            git am --show-current-patch=diff >&2
            git am --abort
            return 1
        fi
    done
    echo "ERROR: patch series did not finish after $attempt retries."
    git am --abort
    return 1
}

if ! git log >/dev/null 2>&1 || ! git log | grep "arm-generic-fdt" >/dev/null; then
    echo "  -> Applying arm-generic-fdt-v3.mbx..."
    apply_patch_series "$WORKSPACE_DIR/patches/arm-generic-fdt-v3.mbx"
else
    echo "  -> arm-generic-fdt patch already applied."
fi

# 2. Allow dynamic loading of SysBus devices via `-device`
if ! grep -q "machine_class_allow_dynamic_sysbus_dev(mc, \"sys-bus-device\")" hw/arm/arm_generic_fdt.c; then
    echo "  -> Enabling dynamic sysbus devices (ARM)..."
    sed 's/mc->minimum_page_bits = 12;/mc->minimum_page_bits = 12;\n\n    \/* virtmcu: allow all SysBus devices via -device; arm-generic-fdt loads devices from DTB at runtime *\/\n    machine_class_allow_dynamic_sysbus_dev(mc, "sys-bus-device");/' hw/arm/arm_generic_fdt.c > hw/arm/arm_generic_fdt.c.tmp && mv hw/arm/arm_generic_fdt.c.tmp hw/arm/arm_generic_fdt.c
fi

if [ -f hw/riscv/virt.c ]; then
    echo "  -> Enabling dynamic sysbus devices (RISC-V)..."
    sed -i 's/MACHINE_CLASS(mc)->has_dynamic_sysbus = false;/MACHINE_CLASS(mc)->has_dynamic_sysbus = true;/' hw/riscv/virt.c
    # virtmcu: allow specific virtmcu devices
    if ! grep -q "virtmcu-clock" hw/riscv/virt.c; then
        sed -i '/machine_class_allow_dynamic_sysbus_dev(mc, TYPE_UEFI_VARS_SYSBUS);/a \    machine_class_allow_dynamic_sysbus_dev(mc, "virtmcu-clock");\n    machine_class_allow_dynamic_sysbus_dev(mc, "mmio-socket-bridge");' hw/riscv/virt.c
    fi
    # virtmcu: add devices to sysbus-fdt bindings so they bypass the FDT generation check
    if ! grep -q "virtmcu-clock" hw/core/sysbus-fdt.c; then
        sed -i '/TYPE_BINDING(TYPE_UEFI_VARS_SYSBUS, add_uefi_vars_node),/a \    TYPE_BINDING("virtmcu-clock", no_fdt_node),\n    TYPE_BINDING("mmio-socket-bridge", no_fdt_node),' hw/core/sysbus-fdt.c
    fi
    # virtmcu: NOTE: We do NOT allow "sys-bus-device" globally on RISC-V because it causes
    # double-mapping of board-default devices (like UART) to the platform bus,
    # leading to Assertion `!subregion->container` failed.
fi

# 3. Update Meson version to support objects in Rust targets
TARGET_MESON_VERSION="${MESON_VERSION:-1.8.0}"
if grep -q "meson_version: '>=" meson.build; then
    CURRENT_MESON_VERSION=$(grep "meson_version: '>=" meson.build | sed -E "s/.*meson_version: '>=([^']+)'.*/\1/")
    echo "  -> Found Meson requirement: ${CURRENT_MESON_VERSION}"
    if [ "$(printf '%s\n%s' "$TARGET_MESON_VERSION" "$CURRENT_MESON_VERSION" | sort -V | head -n1)" = "$CURRENT_MESON_VERSION" ] && [ "$TARGET_MESON_VERSION" != "$CURRENT_MESON_VERSION" ]; then
        echo "  -> Updating Meson requirement to ${TARGET_MESON_VERSION} (required for Rust)..."
        sed -i "s/meson_version: '>=${CURRENT_MESON_VERSION}'/meson_version: '>=${TARGET_MESON_VERSION}'/" meson.build
    fi
fi

# 4. Apply custom Python-based AST-injection patches (Zenoh hooks, etc.)
cd "$WORKSPACE_DIR"
echo "  -> Injecting Zenoh hooks and QAPI extensions..."
python3 patches/apply_zenoh_hook.py "$QEMU_DIR"
python3 patches/apply_zenoh_qapi.py "$QEMU_DIR"
python3 patches/apply_zenoh_netdev.py "$QEMU_DIR"
python3 patches/apply_zenoh_chardev.py "$QEMU_DIR"
python3 patches/apply_fdt_generic_util_fix.py "$QEMU_DIR"
python3 patches/apply_sysbus_asan_fix.py "$QEMU_DIR"
python3 patches/apply_rust_asan_fix.py "$QEMU_DIR"

# 5. Apply the module crash fix
cd "$QEMU_DIR"
if ! grep -q "VIRTMCU-PATCH: error_prepend() crashes" qom/object.c; then
    echo "  -> Applying qemu-module-crash-fix.patch..."
    git apply "$WORKSPACE_DIR/patches/qemu-module-crash-fix.patch"
fi

cd "$WORKSPACE_DIR"
echo "✓ All patches applied successfully."
