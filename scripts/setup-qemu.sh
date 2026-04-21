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

# Check if QEMU is already pre-installed in the container image
if [ -x "/opt/virtmcu/bin/qemu-system-arm" ] && [ "$1" != "--force" ] && [ ! -d "$QEMU_DIR" ]; then
    echo "==> QEMU is already pre-installed in this environment (/opt/virtmcu/bin)."
    echo "    'make run' and integration tests will work immediately."
    echo ""
    echo "    If you want to modify QEMU C code or add new peripherals in hw/,"
    echo "    run: ./scripts/setup-qemu.sh --force"
    echo ""
    exit 0
fi

if [ -f "$WORKSPACE_DIR/VERSIONS" ]; then
    # shellcheck source=/dev/null
    source "$WORKSPACE_DIR/VERSIONS"
fi


# Clone QEMU if not already present
QEMU_REPO="${QEMU_REPO:-https://gitlab.com/qemu-project/qemu.git}"
QEMU_REF="${QEMU_REF:-v${QEMU_VERSION:-11.0.0-rc4}}"

if [ ! -d "$QEMU_DIR/.git" ]; then
    echo "==> Cloning QEMU ${QEMU_REF} from ${QEMU_REPO} ..."
    mkdir -p "$WORKSPACE_DIR/third_party"
    git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" "$QEMU_DIR"
fi

cd "$QEMU_DIR"

# Ensure we are on the expected QEMU version (11.0.0-rc4)
VERSION=$(cat VERSION || echo "")
if [[ "$VERSION" != *"10.2.9"* ]] && [[ "$VERSION" != *"11.0.0-rc"* ]]; then
    echo "Unexpected QEMU version: $VERSION"
    exit 1
fi

# Apply all virtmcu patches (arm-generic-fdt, SysBus, Zenoh hooks)
# We use a centralized script to ensure the Dockerfile and local dev stay 1:1 consistent.
bash "$WORKSPACE_DIR/scripts/apply-qemu-patches.sh" "$QEMU_DIR"

# Phase 7: Fetch Zenoh-C prebuilt library for native QOM plugins
ZENOHC_VER="${ZENOH_VERSION:-1.9.0}"
ZENOHC_DIR="$WORKSPACE_DIR/third_party/zenoh-c"
if [ ! -d "$ZENOHC_DIR/include" ]; then
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
if command -v flatcc >/dev/null 2>&1; then
    echo "==> flatcc already installed in system, skipping local build."
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
