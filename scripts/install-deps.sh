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
#
# DESIGNED FOR DOWNSTREAM REUSE: Automatically resolves paths to support being
# run within a parent project where VirtMCU is embedded.
# ==============================================================================

set -euo pipefail

# Determine absolute paths for the script, workspace, and QEMU directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"
QEMU_DIR="$WORKSPACE_DIR/third_party/qemu"

if [ -f "$WORKSPACE_DIR/BUILD_DEPS" ]; then
    # shellcheck source=/dev/null
    source "$WORKSPACE_DIR/BUILD_DEPS"
fi

# Inherit optional env vars with safe defaults for -u compatibility
CI="${CI:-}"
VIRTMCU_USE_CCACHE="${VIRTMCU_USE_CCACHE:-}"
VIRTMCU_USE_ASAN="${VIRTMCU_USE_ASAN:-}"
VIRTMCU_USE_TSAN="${VIRTMCU_USE_TSAN:-}"

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
bash "$SCRIPTS_DIR/apply-qemu-patches.sh" "$QEMU_DIR"

# Compile Zenoh-C from source for native QOM plugins (guarantees parity with CI)
ZENOHC_VER="${ZENOH_VERSION:-1.9.0}"
if [ "${VIRTMCU_USE_ASAN:-0}" = "1" ]; then
    ZENOHC_DIR="$WORKSPACE_DIR/third_party/zenoh-c-asan"
    export RUSTC_BOOTSTRAP=1
    # Fix for proc-macro compilation with ASan: force separate host/target builds
    TRIPLE="$(rustc -vV | grep 'host:' | awk '{print $2}')"
    export CARGO_BUILD_TARGET="${TRIPLE}"
    TRIPLE_UPPER="$(echo "${TRIPLE}" | tr '[:lower:]-' '[:upper:]_')"
    export "CARGO_TARGET_${TRIPLE_UPPER}_RUSTFLAGS=-Zsanitizer=address"
    export "CFLAGS_${TRIPLE}=-fsanitize=address -fsanitize=undefined -fno-omit-frame-pointer"
    export "CXXFLAGS_${TRIPLE}=-fsanitize=address -fsanitize=undefined -fno-omit-frame-pointer"
    export LDFLAGS="-fsanitize=address -fsanitize=undefined"
    export HOST_CFLAGS=""
    export HOST_CXXFLAGS=""
    export CARGO_HOST_RUSTFLAGS=""
    echo "==> Configuring Zenoh-C for ASan build..."
else
    ZENOHC_DIR="$WORKSPACE_DIR/third_party/zenoh-c"
fi

if [ ! -d "$ZENOHC_DIR/include" ]; then
    echo "==> Compiling Zenoh-C $ZENOHC_VER from source into $ZENOHC_DIR..."
    rm -rf "$ZENOHC_DIR" "/tmp/zenoh-c-src" "/tmp/zenoh-c-build" # virtmcu-allow: absolute_path reasoning="Legacy script"
    git clone --depth=1 --branch "$ZENOHC_VER" \
        https://github.com/eclipse-zenoh/zenoh-c.git /tmp/zenoh-c-src # virtmcu-allow: absolute_path reasoning="Legacy script"
    
    cmake -S /tmp/zenoh-c-src -B /tmp/zenoh-c-build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$ZENOHC_DIR" -DZENOHC_BUILD_WITH_SHARED_MEMORY=OFF # virtmcu-allow: absolute_path reasoning="Legacy script"
       
    cmake --build /tmp/zenoh-c-build -j"$(nproc)" # virtmcu-allow: absolute_path reasoning="Legacy script"
    cmake --install /tmp/zenoh-c-build # virtmcu-allow: absolute_path reasoning="Legacy script"
    
    rm -rf "/tmp/zenoh-c-src" "/tmp/zenoh-c-build" # virtmcu-allow: absolute_path reasoning="Legacy script"
fi

# Compile flatcc from source for Telemetry (guarantees parity with CI)
FLATCC_VER="${FLATCC_VERSION:-0.6.1}"
FLATCC_DIR="$WORKSPACE_DIR/third_party/flatcc"

if [ ! -x "$FLATCC_DIR/bin/flatcc" ]; then
    echo "==> Compiling flatcc v$FLATCC_VER from source..."
    rm -rf "$FLATCC_DIR"
    git clone --depth=1 --branch "v$FLATCC_VER" https://github.com/dvidelabs/flatcc.git "$FLATCC_DIR"
    
    cd "$FLATCC_DIR"
    cmake -B build -G Ninja -Wno-dev \
       -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
       -DFLATCC_TEST=OFF \
       -DFLATCC_CXX_TEST=OFF \
       -DFLATCC_INSTALL=ON \
       -DCMAKE_INSTALL_PREFIX="$FLATCC_DIR/install" \
       -DCMAKE_C_FLAGS="-w"
    
    cmake --build build --target install
    
    # Expose binary and libraries at expected workspace locations
    mkdir -p bin include lib
    cp install/bin/flatcc bin/
    cp -r install/include/flatcc include/
    cp -r install/lib/libflatcc* lib/
    cd "$WORKSPACE_DIR"
fi

# Symlink our custom hw/ directory into QEMU's hw/virtmcu directory
# This allows QEMU's Meson build system to compile our custom peripherals
ln -sfn "$WORKSPACE_DIR/hw" "$QEMU_DIR/hw/virtmcu"
ln -sfn "$WORKSPACE_DIR/Cargo.toml" "$QEMU_DIR/hw/Cargo.toml"
ln -sfn "$WORKSPACE_DIR/Cargo.lock" "$QEMU_DIR/hw/Cargo.lock"

# Configure and build QEMU in a dedicated build directory
cd "$QEMU_DIR"
BUILD_DIR_NAME="build-virtmcu$( [ "$VIRTMCU_USE_ASAN" = "1" ] && echo "-asan" || echo "" )$( [ "$VIRTMCU_USE_TSAN" = "1" ] && echo "-tsan" || echo "" )"
echo "==> QEMU Build Directory: $QEMU_DIR/$BUILD_DIR_NAME"
mkdir -p "$BUILD_DIR_NAME"
cd "$BUILD_DIR_NAME"

# Configure the build, handling macOS specific plugin bugs (GitLab #516)
# Enable --enable-rust for native QOM plugins
# Use LLVM linker (lld) for faster linking
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
    echo "ASAN/UBSAN enabled: adding --enable-asan --enable-ubsan -Db_sanitize=address,undefined to QEMU build"
    CONFIGURE_ARGS+=(--enable-asan --enable-ubsan "-Db_sanitize=address,undefined")
    export VIRTMCU_USE_ASAN
    # Ensure all Rust targets (including QEMU's own and our plugins) link with sanitizers
    export RUSTC_BOOTSTRAP=1
    export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address"
    export HOST_CFLAGS=""
    export HOST_CXXFLAGS=""
elif [ "$VIRTMCU_USE_TSAN" = "1" ]; then
    echo "TSAN enabled: adding --enable-tsan -Db_sanitize=thread to QEMU build"
    CONFIGURE_ARGS+=(--enable-tsan -Db_sanitize=thread)
    export VIRTMCU_USE_TSAN
    # ThreadSanitizer in Rust requires nightly or RUSTC_BOOTSTRAP=1 with unstable flags
    export RUSTC_BOOTSTRAP=1
    export RUSTFLAGS="${RUSTFLAGS:-} -Z sanitizer=thread"
fi

if [ "$(uname)" = "Darwin" ]; then
    echo "macOS detected: disabling --enable-plugins to avoid GLib module conflicts"
else
    CONFIGURE_ARGS+=(--enable-plugins)
    # Check if lld is available
    if command -v lld >/dev/null 2>&1; then
        echo "lld detected: enabling fast linking"
        CONFIGURE_ARGS+=(--extra-ldflags="-fuse-ld=lld -rdynamic")
    fi
fi

../configure "${CONFIGURE_ARGS[@]}"

# Compile QEMU.
JOBS=$(nproc)

make -j"$JOBS"
# Install QEMU binaries to the prefix directory (build-virtmcu/install)
make install
echo "QEMU build and install completed successfully."
