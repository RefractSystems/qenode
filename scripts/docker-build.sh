#!/usr/bin/env bash
# Build and smoke-test virtmcu Docker image stages.
#
# Usage:
#   scripts/docker-build.sh [TARGET] [IMAGE_TAG]
#
#   TARGET    dev (default) | all | base | toolchain | devenv | builder | runtime
#   IMAGE_TAG local tag suffix, default: dev
#
# Examples:
#   scripts/docker-build.sh             # build base → toolchain → devenv, smoke-test each
#   scripts/docker-build.sh all         # same + builder (slow: ~40 min) + runtime
#   scripts/docker-build.sh toolchain   # build a single stage only, no smoke test
#   IMAGE_TAG=ci scripts/docker-build.sh dev
#
# All versions are read from the VERSIONS file at the repo root.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

TARGET="${1:-dev}"
IMAGE_TAG="${IMAGE_TAG:-dev}"

# ── Load versions ──────────────────────────────────────────────────────────────
if [[ ! -f VERSIONS ]]; then
    echo "error: VERSIONS file not found (run from repo root or via make)" >&2
    exit 1
fi
# shellcheck source=../VERSIONS
set -a
# grep strips comments and blank lines; eval-safe because VERSIONS is version strings only
while IFS='=' read -r key val; do
    export "${key}=${val}"
done < <(grep -v '^#' VERSIONS | grep -v '^[[:space:]]*$')
set +a

# ── Build args ─────────────────────────────────────────────────────────────────
DOCKER_BUILD_ARGS=(
    --build-arg "DEBIAN_CODENAME=${DEBIAN_CODENAME}"
    --build-arg "NODE_VERSION=${NODE_VERSION}"
    --build-arg "PYTHON_VERSION=${PYTHON_VERSION}"
    --build-arg "ARM_TOOLCHAIN_VERSION=${ARM_TOOLCHAIN_VERSION}"
    --build-arg "QEMU_REF=v${QEMU_VERSION}"
    --build-arg "ZENOH_C_REF=${ZENOH_VERSION}"
    --build-arg "CMAKE_VERSION=${CMAKE_VERSION}"
    --build-arg "RUST_VERSION=${RUST_VERSION}"
    --build-arg "FLATBUFFERS_VERSION=${FLATBUFFERS_VERSION}"
    --file docker/Dockerfile
    .
)

# ── Helpers ────────────────────────────────────────────────────────────────────
section() { echo ""; echo "══════════════════════════════════════════════════"; echo "  $*"; echo "══════════════════════════════════════════════════"; }
ok()      { echo "  ✓ $*"; }
fail()    { echo "  ✗ $*" >&2; exit 1; }

image_for() { echo "virtmcu-${1}:${IMAGE_TAG}"; }

build_stage() {
    local stage="$1"
    local img
    img="$(image_for "${stage}")"
    section "Building stage: ${stage}  →  ${img}"
    echo "  Debian:  ${DEBIAN_CODENAME}"
    echo "  Python:  ${PYTHON_VERSION}  |  ARM toolchain: ${ARM_TOOLCHAIN_VERSION}"
    echo "  Rust:    ${RUST_VERSION}    |  Node: ${NODE_VERSION}"
    echo ""
    docker build --target "${stage}" --tag "${img}" "${DOCKER_BUILD_ARGS[@]}"
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
        claude --version
        echo '  --- Gemini CLI ---'
        gemini --version 2>/dev/null || gemini --help 2>&1 | head -1
        echo '  --- Rust ---'
        cargo --version
        rustc --version
        echo '  --- ARM toolchain (inherited from toolchain) ---'
        arm-none-eabi-gcc --version | head -1
        echo '  --- uv ---'
        uv --version
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
        # One-stop for local development: base → toolchain → devenv with smoke tests
        build_stage base
        smoke_base
        build_stage toolchain
        smoke_toolchain
        build_stage devenv
        smoke_devenv
        section "All dev stages built and verified"
        echo "  Images ready:"
        echo "    $(image_for base)"
        echo "    $(image_for toolchain)"
        echo "    $(image_for devenv)"
        echo ""
        echo "  Open devcontainer:  use VS Code 'Reopen in Container'"
        echo "  Inspect directly:   docker run --rm -it --user vscode $(image_for devenv) zsh"
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
        echo "  NOTE: builder stage compiles QEMU (~40 min on first run, cached after)"
        build_stage builder
        smoke_builder
        build_stage runtime
        smoke_runtime
        section "All stages built and verified"
        for s in base toolchain devenv builder runtime; do
            echo "    $(image_for "${s}")"
        done
        ;;
    *)
        echo "error: unknown target '${TARGET}'" >&2
        echo "usage: $0 [dev|all|base|toolchain|devenv|builder|runtime]" >&2
        exit 1
        ;;
esac

echo ""
