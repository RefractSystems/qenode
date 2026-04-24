#!/usr/bin/env bash
# Build and smoke-test virtmcu Docker image stages.
#
# Usage:
#   scripts/docker-build.sh [TARGET] [IMAGE_TAG]
#
#   TARGET    dev (default) | all | base | toolchain | devenv-base | devenv | builder | runtime
#   IMAGE_TAG local tag suffix, default: dev
#
# Examples:
#   scripts/docker-build.sh             # build base → toolchain → devenv-base, smoke-test each
#   scripts/docker-build.sh all         # same + builder (slow: ~40 min) + devenv + runtime
#   scripts/docker-build.sh toolchain   # build a single stage only, no smoke test
#   IMAGE_TAG=ci scripts/docker-build.sh dev
#
# All versions are read from the BUILD_DEPS file at the repo root.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

TARGET="${1:-dev}"
IMAGE_TAG="${IMAGE_TAG:-dev}"

# ── Load versions ──────────────────────────────────────────────────────────────
if [[ ! -f BUILD_DEPS ]]; then
    echo "error: BUILD_DEPS file not found (run from repo root or via make)" >&2
    exit 1
fi
# shellcheck source=../BUILD_DEPS
set -a
# grep strips comments and blank lines; eval-safe because BUILD_DEPS is version strings only
while IFS='=' read -r key val; do
    export "${key}=${val}"
done < <(grep -v '^#' BUILD_DEPS | grep -v '^[[:space:]]*$')
set +a

# ── Helpers ────────────────────────────────────────────────────────────────────
section() { echo ""; echo "══════════════════════════════════════════════════"; echo "  $*"; echo "══════════════════════════════════════════════════"; }
ok()      { echo "  ✓ $*"; }
fail()    { echo "  ✗ $*" >&2; exit 1; }

image_for() { 
    local ARCH
    ARCH=$(uname -m)
    if [ "$ARCH" = "x86_64" ]; then ARCH="amd64"; elif [ "$ARCH" = "aarch64" ]; then ARCH="arm64"; fi
    echo "ghcr.io/refractsystems/virtmcu/${1}:${IMAGE_TAG}-${ARCH}" 
}

build_stage() {
    local stage="$1"
    local img
    img="$(image_for "${stage}")"
    section "Building stage: ${stage}  →  ${img}"

    local ARCH
    ARCH=$(uname -m)
    if [ "$ARCH" = "x86_64" ]; then ARCH="amd64"; elif [ "$ARCH" = "aarch64" ]; then ARCH="arm64"; fi

    local USE_CCACHE="${VIRTMCU_USE_CCACHE:-false}"
    if [ "$USE_CCACHE" = "1" ]; then USE_CCACHE="true"; fi

    # Warm up local cache if the image already exists in the registry.
    # This provides a secondary cache layer in case the registry-cache manifests are missing.
    echo "  --> Attempting to pull ${img} to warm up local cache..."
    docker pull "${img}" || echo "      (Pull failed or image not found, proceeding with build)"

    # Use Docker Bake for consistent builds (reads docker-bake.hcl)
    # --load: loads the built image into the local Docker daemon
    ARCH="${ARCH}" IMAGE_TAG="${IMAGE_TAG}" USE_CCACHE="${USE_CCACHE}" \
    docker buildx bake "${stage}" --load
    
    ok "Built ${img}"
}

# ── Smoke tests ────────────────────────────────────────────────────────────────

smoke_base() {
    local img; img="$(image_for base)"
    section "Smoke test: base"
    docker run --rm "${img}" bash -c "
        set -e
        echo '  --- user ---'
        id vscode
        echo '  --- sudo ---'
        sudo -n true
        echo '  --- shell ---'
        zsh --version
        test -d /home/vscode/.oh-my-zsh || (echo 'oh-my-zsh missing' && exit 1)
        echo '  --- locale ---'
        locale | grep 'LANG=en_US.UTF-8'
        echo '  --- uv ---'
        uv --version
        echo '  --- gh ---'
        gh --version | head -1
    "
    ok "base smoke test passed"
}

smoke_toolchain() {
    local img; img="$(image_for toolchain)"
    section "Smoke test: toolchain"
    docker run --rm "${img}" bash -c "
        set -e
        echo '  --- ARM cross-compiler ---'
        arm-none-eabi-gcc --version | head -1
        echo '  --- RISC-V cross-compiler ---'
        riscv64-linux-gnu-gcc --version | head -1
        echo '  --- Python (uv-pinned) ---'
        uv run --python ${PYTHON_VERSION} python --version
        echo '  --- CMake ---'
        cmake --version | head -1
        echo '  --- FlatBuffers compiler ---'
        flatc --version
        echo '  --- meson ---'
        meson --version
    "
    ok "toolchain smoke test passed"
}

smoke_devenv_base() {
    local img; img="$(image_for devenv-base)"
    section "Smoke test: devenv-base"
    # Run as vscode — the expected interactive user
    docker run --rm --user vscode "${img}" bash -c "
        set -e
        echo '  --- Node.js ---'
        node --version
        npm --version
        echo '  --- Claude Code ---'
        claude --version || echo 'claude not installed (expected, handled by post-create.sh)'
        echo '  --- Gemini CLI ---'
        (gemini --version 2>/dev/null || gemini --help 2>&1 | head -1) || echo 'gemini not installed (expected, handled by post-create.sh)'
        echo '  --- Rust ---'
        cargo --version
        rustc --version
        echo '  --- ARM toolchain (inherited from toolchain) ---'
        arm-none-eabi-gcc --version | head -1
        echo '  --- uv ---'
        uv --version
    "
    ok "devenv-base smoke test passed"
}

smoke_devenv() {
    local img; img="$(image_for devenv)"
    section "Smoke test: devenv"
    docker run --rm --user vscode "${img}" bash -c "
        set -e
        echo '  --- QEMU binary (added in devenv) ---'
        qemu-system-arm --version
    "
    ok "devenv smoke test passed"
}

smoke_builder() {
    local img; img="$(image_for builder)"
    section "Smoke test: builder"
    docker run --rm "${img}" bash -c "
        set -e
        echo '  --- QEMU binary ---'
        qemu-system-arm --version
        qemu-system-riscv32 --version | head -1
        qemu-system-riscv64 --version | head -1
        echo '  --- zenoh-c library ---'
        ls -lh /opt/virtmcu/lib/libzenohc.so
        echo '  --- QEMU modules ---'
        ls \${QEMU_MODULE_DIR}/*.so | head -5
    "
    ok "builder smoke test passed"
}

smoke_runtime() {
    local img; img="$(image_for runtime)"
    section "Smoke test: runtime"
    docker run --rm "${img}" bash -c "
        set -e
        echo '  --- QEMU binary ---'
        qemu-system-arm --version
        echo '  --- Python tooling ---'
        python3 -c 'import zenoh; print(\"zenoh:\", zenoh.__version__)'
        python3 -c 'import flatbuffers; print(\"flatbuffers:\", flatbuffers.__version__)'
        echo '  --- tools ---'
        ls /app/tools/
    "
    ok "runtime smoke test passed"
}

# ── Dispatch ───────────────────────────────────────────────────────────────────

echo ""
echo "virtmcu docker-build  |  target=${TARGET}  tag=${IMAGE_TAG}"
echo "  Versions: Debian=${DEBIAN_CODENAME}  QEMU=${QEMU_VERSION}  Zenoh=${ZENOH_VERSION}"

case "${TARGET}" in
    base)
        build_stage base
        ;;
    toolchain)
        build_stage toolchain
        ;;
    devenv-base)
        build_stage devenv-base
        ;;
    devenv)
        build_stage devenv
        ;;
    builder)
        build_stage builder
        ;;
    runtime)
        build_stage runtime
        ;;
    dev)
        # One-stop for local development: base → toolchain → devenv-base with smoke tests
        # This provides a full tool-rich environment but skips the slow QEMU build.
        build_stage base
        smoke_base
        build_stage toolchain
        smoke_toolchain
        build_stage devenv-base
        smoke_devenv_base
        section "All dev-base stages built and verified"
        echo "  Images ready:"
        echo "    $(image_for base)"
        echo "    $(image_for toolchain)"
        echo "    $(image_for devenv-base)"
        echo ""
        echo "  To build QEMU locally:  scripts/docker-build.sh devenv"
        ;;
    all)
        # Full pipeline including the slow QEMU build
        build_stage base
        smoke_base
        build_stage toolchain
        smoke_toolchain
        build_stage devenv-base
        smoke_devenv_base
        echo ""
        echo "  NOTE: builder stage compiles QEMU (~40 min on first run, cached after)"
        build_stage builder
        smoke_builder
        build_stage devenv
        smoke_devenv
        build_stage runtime
        smoke_runtime
        section "All stages built and verified"
        for s in base toolchain devenv-base builder devenv runtime; do
            echo "    $(image_for "${s}")"
        done
        ;;
    *)
        echo "error: unknown target '${TARGET}'" >&2
        echo "usage: $0 [dev|all|base|toolchain|devenv-base|devenv|builder|runtime]" >&2
        exit 1
        ;;
esac

echo ""
