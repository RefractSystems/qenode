#!/usr/bin/env bash
# Build and integration virtmcu Docker image stages.
#
# Usage:
#   scripts/docker-build.sh [TARGET] [IMAGE_TAG]
#
#   TARGET    dev (default) | all | base | toolchain | devenv | ci | ci-asan
#   IMAGE_TAG local tag suffix, default: dev
#
# Examples:
#   scripts/docker-build.sh             # build base → toolchain → devenv each
#   scripts/docker-build.sh all         # same + ci (slow: ~40 min)
#   scripts/docker-build.sh toolchain   # build a single stage only, no integration test
#   IMAGE_TAG=ci scripts/docker-build.sh dev
#
# All versions are read from the BUILD_DEPS file at the repo root.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

TARGET="${1:-dev}"

# Dynamic IMAGE_TAG logic matching Makefile:
if [ -z "${IMAGE_TAG:-}" ]; then
    GIT_EXACT_TAG=$(git describe --tags --exact-match 2>/dev/null || true)
    if [ -n "$GIT_EXACT_TAG" ]; then
        IMAGE_TAG=$(echo "$GIT_EXACT_TAG" | sed 's/^v//')
    elif [ "${CI:-false}" = "true" ]; then
        IMAGE_TAG="sha-$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
    else
        IMAGE_TAG="latest"
    fi
fi
export IMAGE_TAG

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

# ── Derived versions ───────────────────────────────────────────────────────────
PATCHES_HASH=$( (cat BUILD_DEPS; find patches -type f | sort | xargs cat) | sha256sum | head -c 12 )
export THIRD_PARTY_CACHE_TAG="${QEMU_VERSION}-${PATCHES_HASH}"

image_for() { 
    local ARCH
    ARCH=$(uname -m)
    if [ "$ARCH" = "x86_64" ]; then ARCH="amd64"; elif [ "$ARCH" = "aarch64" ]; then ARCH="arm64"; fi
    
    local stage="${1}"
    local tag="${IMAGE_TAG}"
    local package=""
    
    # Map internal stages to public image names from BUILD_DEPS
    case "${stage}" in
        devenv)
            package="${VIRTMCU_DEVENV_IMAGE}"
            ;;
        ci)
            package="${VIRTMCU_CI_IMAGE}"
            ;;
        ci-asan)
            package="${VIRTMCU_CI_IMAGE}"
            tag="${IMAGE_TAG}-asan"
            ;;
        runtime)
            package="runtime"
            ;;
        third-party-base-asan)
            package="third-party-base"
            tag="${THIRD_PARTY_CACHE_TAG}-asan"
            ;;
        third-party-base)
            package="third-party-base"
            tag="${THIRD_PARTY_CACHE_TAG}"
            ;;
        *)
            # For base, toolchain, flatcc-builder, etc.
            package="${stage}"
            ;;
    esac
    
    echo "${VIRTMCU_IMAGE_REGISTRY}/${package}:${tag}-${ARCH}" 
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
    VIRTMCU_USE_ASAN="${VIRTMCU_USE_ASAN:-0}" THIRD_PARTY_CACHE_TAG="${THIRD_PARTY_CACHE_TAG}" \
    VIRTMCU_IMAGE_REGISTRY="${VIRTMCU_IMAGE_REGISTRY}" VIRTMCU_DEVENV_IMAGE="${VIRTMCU_DEVENV_IMAGE}" VIRTMCU_CI_IMAGE="${VIRTMCU_CI_IMAGE}" \
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
        test -d /home/vscode/.oh-my-zsh || (echo 'oh-my-zsh missing' && exit 1) # virtmcu-allow: absolute_path reasoning='Legacy script'
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

smoke_devenv() {
    local img; img="$(image_for devenv)"
    section "Smoke test: devenv"
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
        echo '  --- mdbook ---'
        mdbook --version
        mdbook-mermaid --version
        mdbook-pdf --version
        echo '  --- chromium ---'
        chromium --version
        echo '  --- ARM toolchain (inherited from toolchain) ---'
        arm-none-eabi-gcc --version | head -1
        echo '  --- uv ---'
        uv --version
    "
    ok "devenv smoke test passed"
}

smoke_ci() {
    local img; img="$(image_for ci)"
    section "Smoke test: ci"
    docker run --rm "${img}" bash -c "
        set -e
        echo '  --- QEMU binary ---'
        qemu-system-arm --version
        qemu-system-riscv32 --version | head -1
        qemu-system-riscv64 --version | head -1
        echo '  --- zenoh-c library ---'
        echo '  --- QEMU modules ---'
        ls \${QEMU_MODULE_DIR}/*.so | head -5
    "
    ok "ci smoke test passed"
}

smoke_ci_asan() {
    local img; img="$(image_for ci-asan)"
    section "Smoke test: ci-asan"
    docker run --rm "${img}" bash -c "
        set -e
        echo '  --- QEMU binary (ASan) ---'
        qemu-system-arm --version
        echo '  --- check for ASan symbols ---'
        nm /build/qemu/build-virtmcu/install/bin/qemu-system-arm | grep -q __asan || (echo 'ASan symbols NOT found' && exit 1)
    "
    ok "ci-asan smoke test passed"
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
    devenv)
        build_stage devenv
        ;;
    ci)
        build_stage ci
        ;;
    ci-asan)
        build_stage ci-asan
        ;;
    third-party-builder)
        if [ "${VIRTMCU_USE_ASAN:-0}" = "1" ]; then
            build_stage third-party-base-asan
        else
            build_stage third-party-base
        fi
        ;;
    dev)
        # One-stop for local development: base → toolchain → devenv with smoke tests
        # This provides a full tool-rich environment but skips the slow QEMU build.
        build_stage base
        smoke_base
        build_stage toolchain
        smoke_toolchain
        build_stage devenv
        smoke_devenv
        section "All dev-base stages built and verified"
        echo "  Images ready:"
        echo "    $(image_for base)"
        echo "    $(image_for toolchain)"
        echo "    $(image_for devenv)"
        echo ""
        echo "  To build QEMU locally:  scripts/docker-build.sh ci"
        ;;
    all)
        # Full pipeline including the slow QEMU build
        build_stage base
        smoke_base
        build_stage toolchain
        smoke_toolchain
        build_stage devenv
        smoke_devenv
        echo ""
        echo "  NOTE: ci stage compiles QEMU (~40 min on first run, cached after)"
        build_stage ci
        smoke_ci
        build_stage ci-asan
        smoke_ci_asan
        build_stage runtime
        section "All stages built and verified"
        for s in base toolchain devenv ci ci-asan runtime; do
            echo "    $(image_for "${s}")"
        done
        ;;
    runtime)
        build_stage runtime
        ;;
    *)
        echo "error: unknown target '${TARGET}'" >&2
        echo "usage: $0 [dev|all|base|toolchain|devenv|third-party-builder|ci|ci-asan|runtime]" >&2
        exit 1
        ;;
esac

echo ""
