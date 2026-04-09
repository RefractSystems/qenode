#!/usr/bin/env bash
# setup-qemu.sh — clone QEMU, apply patch series, integrate qenode hw/, and build.
#
# Run once per machine, or re-run after pulling new patches.
# Idempotent: safe to run multiple times.
#
# Environment variables:
#   QEMU_SRC   Path to QEMU source tree (default: third_party/qemu)
#   QEMU_BUILD Path for out-of-tree build dir (default: $QEMU_SRC/build-qenode)
#   JOBS       Parallel build jobs (default: nproc / sysctl)

set -euo pipefail


SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

QEMU_SRC="${QEMU_SRC:-$REPO_ROOT/third_party/qemu}"
QEMU_BUILD="${QEMU_BUILD:-$QEMU_SRC/build-qenode}"
RENODE_SRC="${RENODE_SRC:-$REPO_ROOT/third_party/renode}"
PATCHES_DIR="$REPO_ROOT/patches"

OS="$(uname -s)"
case "$OS" in
  Linux)  JOBS="${JOBS:-$(nproc)}" ;;
  Darwin) JOBS="${JOBS:-$(sysctl -n hw.logicalcpu)}" ;;
  *)      JOBS="${JOBS:-4}" ;;
esac

echo "==> qenode setup"
echo "    QEMU_SRC  : $QEMU_SRC"
echo "    QEMU_BUILD: $QEMU_BUILD"
echo "    OS        : $OS  (jobs=$JOBS)"
echo ""

# ── 1. Clone QEMU if not present ─────────────────────────────────────────────
if [ ! -d "$QEMU_SRC/.git" ]; then
  echo "==> Cloning QEMU..."
  git clone https://gitlab.com/qemu-project/qemu.git "$QEMU_SRC"
  cd "$QEMU_SRC"
  git submodule update --init --recursive
fi

if [ ! -d "$RENODE_SRC/.git" ]; then
  echo "==> Cloning Renode (for test assets)..."
  git clone https://github.com/renode/renode.git "$RENODE_SRC"
fi

cd "$QEMU_SRC"

# ── 2. Apply arm-generic-fdt patch series ────────────────────────────────────
PATCH_BRANCH="qenode-patches"
ARM_FDT_MBX="$PATCHES_DIR/arm-generic-fdt-v3.mbx"

if ! git rev-parse --verify "$PATCH_BRANCH" > /dev/null 2>&1; then
  echo "==> Creating patch branch '$PATCH_BRANCH' from current HEAD..."
  BASE_COMMIT="$(git rev-parse HEAD)"
  git checkout -b "$PATCH_BRANCH"

  echo "==> Applying arm-generic-fdt v3 (33 patches)..."
  if [ ! -f "$ARM_FDT_MBX" ]; then
    echo "ERROR: $ARM_FDT_MBX not found."
    echo "       Run: cd $REPO_ROOT && b4 am 20260402215629.745866-1-ruslichenko.r@gmail.com"
    echo "       Then copy the .mbx to $PATCHES_DIR/arm-generic-fdt-v3.mbx"
    exit 1
  fi
  git am --3way "$ARM_FDT_MBX"

  echo "==> Applying libqemu clock-socket extension..."
  python3 "$PATCHES_DIR/apply_libqemu.py" "$QEMU_SRC"

  if [ -f "$PATCHES_DIR/apply_zenoh_hook.py" ]; then
    echo "==> Applying TCG quantum hook extension..."
    python3 "$PATCHES_DIR/apply_zenoh_hook.py" "$QEMU_SRC"
  fi

  echo "==> Patch branch created at $(git rev-parse --short HEAD)"
else
  echo "==> Patch branch '$PATCH_BRANCH' already exists — skipping patch application."
  git checkout "$PATCH_BRANCH"
fi

# ── 3. Link qenode hw/ into QEMU source tree ─────────────────────────────────
QENODE_HW_LINK="$QEMU_SRC/hw/qenode"
if [ ! -L "$QENODE_HW_LINK" ] && [ ! -d "$QENODE_HW_LINK" ]; then
  echo "==> Linking $REPO_ROOT/hw  →  $QENODE_HW_LINK"
  ln -s "$REPO_ROOT/hw" "$QENODE_HW_LINK"
fi

# Append `subdir('qenode')` to hw/meson.build if not already there.
HW_MESON="$QEMU_SRC/hw/meson.build"
if ! grep -q "subdir('qenode')" "$HW_MESON"; then
  echo "==> Adding subdir('qenode') to hw/meson.build"
  echo "" >> "$HW_MESON"
  echo "# qenode out-of-tree peripheral models" >> "$HW_MESON"
  echo "subdir('qenode')" >> "$HW_MESON"
fi

# ── 4. Configure QEMU ─────────────────────────────────────────────────────────
mkdir -p "$QEMU_BUILD"

# macOS: omit --enable-plugins (known breakage with modules on macOS, GitLab #516)
# Linux: include --enable-plugins for TCG instrumentation support
case "$OS" in
  Darwin)
    EXTRA_FLAGS=""
    echo "==> macOS: building without --enable-plugins (see GitLab #516)"
    ;;
  Linux)
    EXTRA_FLAGS="--enable-plugins"
    ;;
  *)
    EXTRA_FLAGS=""
    ;;
esac

echo "==> Configuring QEMU..."
cd "$QEMU_SRC"
./configure \
  --prefix="$QEMU_BUILD/install" \
  --bindir="$QEMU_BUILD/install/bin" \
  --target-list="arm-softmmu,aarch64-softmmu" \
  --enable-modules \
  --enable-fdt \
  --enable-debug \
  --disable-werror \
  $EXTRA_FLAGS \
  --extra-cflags="-I$QEMU_SRC/include" \
  2>&1 | tail -20

# ── 5. Build ──────────────────────────────────────────────────────────────────
echo "==> Building QEMU (jobs=$JOBS)..."
make -C "$QEMU_SRC" -j"$JOBS"

echo "==> Installing to $QEMU_BUILD/install..."
make -C "$QEMU_SRC" install

echo ""
echo "✓ Build complete."
echo "  qemu-system-arm : $QEMU_BUILD/install/bin/qemu-system-arm"
echo "  module dir      : $QEMU_BUILD/install/lib/qemu/"
echo ""
echo "  Add to PATH:  export PATH=\"$QEMU_BUILD/install/bin:\$PATH\""
echo "  Or use:       $REPO_ROOT/scripts/run.sh [args]"
