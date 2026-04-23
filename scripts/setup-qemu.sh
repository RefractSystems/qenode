#!/usr/bin/env bash
# ==============================================================================
# setup-qemu.sh
#
# This script initializes, patches, configures, and builds the QEMU emulator
# used by the virtmcu project. It performs the following steps:
#   1. Clones QEMU (--depth=1) into third_party/qemu if not already present.
#   2. Applies the 'arm-generic-fdt' patch series via `git am`.
#   3. Applies custom AST-injection patches (zenoh hooks) to QEMU C code.
#   4. Symlinks the project's custom `hw/` directory into QEMU's build tree.
#   5. Configures QEMU (handling macOS specific flags if necessary).
#   6. Compiles and installs the QEMU binaries to `third_party/qemu/build-virtmcu/install`.
# ==============================================================================

set -e

# Determine absolute paths for the script, workspace, and QEMU directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"

if [ -f "$WORKSPACE_DIR/VERSIONS" ]; then
    # shellcheck source=/dev/null
    source "$WORKSPACE_DIR/VERSIONS"
fi

# Function to download pre-built QEMU SDK from GitHub Releases
download_prebuilt_qemu() {
    local ARCH
    ARCH=$(uname -m)
    if [ "$ARCH" = "x86_64" ]; then ARCH="amd64"; elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then ARCH="arm64"; fi
    
    local TAG="v${QEMU_VERSION}"
    local TARBALL="virtmcu-qemu-${QEMU_VERSION}-${ARCH}.tar.gz"
    local URL="https://github.com/refractsystems/virtmcu/releases/download/${TAG}/${TARBALL}"
    
    echo "==> Attempting to download pre-built QEMU ${TAG} for ${ARCH}..."
    if curl -L --head --silent --fail "$URL" > /dev/null; then
        mkdir -p "/tmp/virtmcu-download"
        curl -L "$URL" -o "/tmp/virtmcu-download/${TARBALL}"
        sudo mkdir -p /opt/virtmcu
        sudo tar -xzf "/tmp/virtmcu-download/${TARBALL}" -C /opt/virtmcu
        rm -rf "/tmp/virtmcu-download"
        echo "✓ Pre-built QEMU installed to /opt/virtmcu"
        return 0
    else
        echo "    (No pre-built binary found for ${TAG} at ${URL}, falling back to source build)"
        return 1
    fi
}

# Check if QEMU is already pre-installed in the container image or download it
if [ -x "/opt/virtmcu/bin/qemu-system-arm" ] && [ "$1" != "--force" ] && [ ! -d "$QEMU_DIR" ]; then
    INSTALLED_VER=$(/opt/virtmcu/bin/qemu-system-arm --version 2>/dev/null | awk 'NR==1{print $NF}' || echo "unknown")
    TARGET_VER="${QEMU_VERSION:-11.0.0}"
    if [ "$INSTALLED_VER" = "$TARGET_VER" ]; then
        echo "==> QEMU ${INSTALLED_VER} is pre-installed (/opt/virtmcu/bin). Ready to use."
        echo "    If you want to modify QEMU C code or add new peripherals in hw/,"
        echo "    run: ./scripts/setup-qemu.sh --force"
        echo ""
        FORCE_SYMLINKS=1
    else
        echo "==> WARNING: Pre-installed QEMU ${INSTALLED_VER} differs from target ${TARGET_VER}."
        echo "    This container image was built before the upgrade. Attempting auto-upgrade..."
        if download_prebuilt_qemu; then
            echo "    ✓ Upgraded to QEMU ${TARGET_VER}."
            FORCE_SYMLINKS=1
        else
            echo "    Could not auto-upgrade. Continuing with QEMU ${INSTALLED_VER}."
            echo "    Simulation may behave differently. To rebuild from source:"
            echo "      ./scripts/setup-qemu.sh --force"
            FORCE_SYMLINKS=1
        fi
    fi
elif [ ! -x "/opt/virtmcu/bin/qemu-system-arm" ] && [ "$1" != "--force" ] && [ ! -d "$QEMU_DIR" ] && download_prebuilt_qemu; then
    FORCE_SYMLINKS=1
fi

if [ "${FORCE_SYMLINKS:-0}" != "1" ] || [ -d "$QEMU_DIR" ]; then
    # Clone QEMU if not already present
    QEMU_REPO="${QEMU_REPO:-https://gitlab.com/qemu-project/qemu.git}"
    QEMU_REF="${QEMU_REF:-v${QEMU_VERSION:-11.0.0}}"

    if [ ! -d "$QEMU_DIR/.git" ]; then
        echo "==> Cloning QEMU ${QEMU_REF} from ${QEMU_REPO} ..."
        mkdir -p "$WORKSPACE_DIR/third_party"
        git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" "$QEMU_DIR"
    fi

    cd "$QEMU_DIR"

    # Ensure we are on the expected QEMU version
    VERSION=$(cat VERSION || echo "")
    if [[ "$VERSION" != "${QEMU_VERSION:-11.0.0}" ]]; then
        echo "Unexpected QEMU version: $VERSION (expected ${QEMU_VERSION:-11.0.0})"
        exit 1
    fi

    # Apply all virtmcu patches (arm-generic-fdt, SysBus, Zenoh hooks)
    # We use a centralized script to ensure the Dockerfile and local dev stay 1:1 consistent.
    bash "$WORKSPACE_DIR/scripts/apply-qemu-patches.sh" "$QEMU_DIR"
fi

# Phase 7: Fetch Zenoh-C prebuilt library for native QOM plugins
ZENOHC_VER="${ZENOH_VERSION:-1.9.0}"
ZENOHC_DIR="$WORKSPACE_DIR/third_party/zenoh-c"

# Try to find pre-installed Zenoh-C headers first
if [ ! -d "$ZENOHC_DIR/include" ] && [ -f "/opt/virtmcu/include/zenoh.h" ]; then
    echo "==> Found pre-installed Zenoh-C headers in /opt/virtmcu, creating symlinks..."
    mkdir -p "$ZENOHC_DIR"
    ln -sfn /opt/virtmcu/include "$ZENOHC_DIR/include"
    ln -sfn /opt/virtmcu/lib "$ZENOHC_DIR/lib"
elif [ ! -d "$ZENOHC_DIR/include" ]; then
    echo "==> Fetching Zenoh-C $ZENOHC_VER for native QEMU plugins..."
    ARCH=$(uname -m)
    if [ "$ARCH" = "x86_64" ]; then
        ZENOHC_URL="https://github.com/eclipse-zenoh/zenoh-c/releases/download/${ZENOHC_VER}/zenoh-c-${ZENOHC_VER}-x86_64-unknown-linux-gnu-standalone.zip"
    elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then
        ZENOHC_URL="https://github.com/eclipse-zenoh/zenoh-c/releases/download/${ZENOHC_VER}/zenoh-c-${ZENOHC_VER}-aarch64-unknown-linux-gnu-standalone.zip"
    else
        echo "Unsupported architecture for prebuilt Zenoh-C: $ARCH"
        exit 1
    fi
    mkdir -p "$ZENOHC_DIR"
    curl -L "$ZENOHC_URL" -o /tmp/zenoh-c.zip
    unzip -q /tmp/zenoh-c.zip -d "$ZENOHC_DIR"
    rm /tmp/zenoh-c.zip
fi

# Phase 12: Fetch and compile flatcc for Telemetry
FLATCC_DIR="$WORKSPACE_DIR/third_party/flatcc"
# Try to find pre-installed flatcc headers first
if [ ! -x "$FLATCC_DIR/bin/flatcc" ] && [ -f "/usr/local/bin/flatcc" ] && [ -d "/usr/local/include/flatcc" ]; then
    echo "==> Found pre-installed flatcc in /usr/local, creating symlinks..."
    mkdir -p "$FLATCC_DIR"
    mkdir -p "$FLATCC_DIR/bin"
    ln -sfn /usr/local/bin/flatcc "$FLATCC_DIR/bin/flatcc"
    ln -sfn /usr/local/include/flatcc "$FLATCC_DIR/include"
    ln -sfn /usr/local/lib "$FLATCC_DIR/lib"
elif [ ! -x "$FLATCC_DIR/bin/flatcc" ] && [ -f "/opt/virtmcu/bin/flatcc" ] && [ -d "/opt/virtmcu/include/flatcc" ]; then
    echo "==> Found pre-installed flatcc in /opt/virtmcu, creating symlinks..."
    mkdir -p "$FLATCC_DIR"
    mkdir -p "$FLATCC_DIR/bin"
    ln -sfn /opt/virtmcu/bin/flatcc "$FLATCC_DIR/bin/flatcc"
    ln -sfn /opt/virtmcu/include/flatcc "$FLATCC_DIR/include"
    ln -sfn /opt/virtmcu/lib "$FLATCC_DIR/lib"
elif command -v flatcc >/dev/null 2>&1 && [ ! -x "$FLATCC_DIR/bin/flatcc" ]; then
    echo "==> flatcc already installed in system PATH, skipping local build."
elif [ ! -x "$FLATCC_DIR/bin/flatcc" ]; then
    echo "==> Fetching and compiling flatcc (v${FLATCC_VERSION})..."
    mkdir -p "$WORKSPACE_DIR/third_party"
    git clone --depth=1 --branch "v${FLATCC_VERSION}" https://github.com/dvidelabs/flatcc.git "$FLATCC_DIR"
    cd "$FLATCC_DIR"
    # Patch CMakeLists.txt to require CMake 3.5+ for compatibility with modern CMake (4.x)
    # Modern CMake removes support for very old (2.8) versions.
    if [ "$(uname)" = "Darwin" ]; then
        sed -i '' 's/cmake_minimum_required (VERSION 2.8)/cmake_minimum_required (VERSION 3.5)/' CMakeLists.txt
    else
        sed -i 's/cmake_minimum_required (VERSION 2.8)/cmake_minimum_required (VERSION 3.5)/' CMakeLists.txt
    fi
    FLATCC_BUILD_FLAGS="-DFLATCC_TEST=OFF -DFLATCC_CXX_TEST=OFF -Wno-dev" CFLAGS="-fPIC" ./scripts/build.sh
    cd "$WORKSPACE_DIR"
fi

if [ "${FORCE_SYMLINKS:-0}" = "1" ] && [ ! -d "$QEMU_DIR" ]; then
    echo "==> Environment ready (using /opt/virtmcu). Skipping QEMU build."
    exit 0
fi

# Symlink our custom hw/ directory into QEMU's hw/virtmcu directory
# This allows QEMU's Meson build system to compile our custom peripherals
ln -sfn "$WORKSPACE_DIR/hw" "$QEMU_DIR/hw/virtmcu"
ln -sfn "$WORKSPACE_DIR/Cargo.toml" "$QEMU_DIR/hw/Cargo.toml"
ln -sfn "$WORKSPACE_DIR/Cargo.lock" "$QEMU_DIR/hw/Cargo.lock"
# Inject 'subdir('virtmcu')' into QEMU's hw/meson.build if not already there
if ! grep -q "subdir('virtmcu')" "$QEMU_DIR/hw/meson.build"; then
    echo "subdir('virtmcu')" >> "$QEMU_DIR/hw/meson.build"
fi

# Configure and build QEMU in a dedicated build directory
cd "$QEMU_DIR"
mkdir -p build-virtmcu
cd build-virtmcu

# Configure the build, handling macOS specific plugin bugs (GitLab #516)
# Phase 18: Enable --enable-rust for native QOM plugins
# Phase 31: Use LLVM linker (lld) for faster linking
CONFIGURE_ARGS=(
    --enable-rust
    --enable-modules
    --enable-fdt
    --enable-debug
    --enable-gcov
    "--target-list=arm-softmmu,riscv32-softmmu,riscv64-softmmu"
    --prefix="$(pwd)/install"
)

if [ "$VIRTMCU_USE_CCACHE" = "1" ]; then
    if command -v ccache >/dev/null 2>&1; then
        echo "ccache enabled: adding --enable-ccache to QEMU build"
        CONFIGURE_ARGS+=(--enable-ccache)
        export CCACHE_SLOPPINESS=time_macros,include_file_mtime
    else
        echo "WARNING: VIRTMCU_USE_CCACHE=1 but 'ccache' command not found. Ignoring."
    fi
fi

if [ "$VIRTMCU_USE_ASAN" = "1" ]; then
    echo "ASAN/UBSAN enabled: adding --enable-asan --enable-ubsan to QEMU build"
    CONFIGURE_ARGS+=(--enable-asan --enable-ubsan)
    export VIRTMCU_USE_ASAN
    # Ensure all Rust targets (including QEMU's own and our plugins) link with sanitizers
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-fsanitize=address -C link-arg=-fsanitize=undefined"
fi

if [ "$(uname)" = "Darwin" ]; then
    echo "macOS detected: disabling --enable-plugins to avoid GLib module conflicts"
else
    CONFIGURE_ARGS+=(--enable-plugins)
    # Check if lld is available
    if command -v lld >/dev/null 2>&1; then
        echo "lld detected: enabling fast linking"
        CONFIGURE_ARGS+=(--extra-ldflags="-fuse-ld=lld")
    fi
fi

../configure "${CONFIGURE_ARGS[@]}"

# Compile QEMU using all available CPU cores
make -j"$(nproc)"
# Install QEMU binaries to the prefix directory (build-virtmcu/install)
make install
echo "QEMU build and install completed successfully."
