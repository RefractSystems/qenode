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

if [ -f hw/riscv/virt.c ] && ! grep -q "machine_class_allow_dynamic_sysbus_dev(mc, \"sys-bus-device\")" hw/riscv/virt.c; then
    echo "  -> Enabling dynamic sysbus devices (RISC-V)..."
    # Inject after TYPE_TPM_TIS_SYSBUS if it exists, otherwise just append to the end of the init function
    if grep -q "machine_class_allow_dynamic_sysbus_dev(mc, TYPE_TPM_TIS_SYSBUS);" hw/riscv/virt.c; then
        sed 's/machine_class_allow_dynamic_sysbus_dev(mc, TYPE_TPM_TIS_SYSBUS);/machine_class_allow_dynamic_sysbus_dev(mc, TYPE_TPM_TIS_SYSBUS);\n    \/* virtmcu: allow all SysBus devices via -device *\/\n    machine_class_allow_dynamic_sysbus_dev(mc, "sys-bus-device");/' hw/riscv/virt.c > hw/riscv/virt.c.tmp && mv hw/riscv/virt.c.tmp hw/riscv/virt.c
    else
        # Fallback: find virt_machine_class_init and inject before it ends
        sed '/static void virt_machine_class_init(ObjectClass *oc, void *data)/,/}/{ /}/i\    machine_class_allow_dynamic_sysbus_dev(mc, "sys-bus-device");\n }' hw/riscv/virt.c > hw/riscv/virt.c.tmp && mv hw/riscv/virt.c.tmp hw/riscv/virt.c
    fi
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
