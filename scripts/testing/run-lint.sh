#!/usr/bin/env bash
# ==============================================================================
# scripts/testing/run-lint.sh - Unified Lint Runner
#
# Runs all linting and static analysis checks.
# Supports running on submodules or parent repositories.
# ==============================================================================
set -euo pipefail

# Find VIRTMCU root (where the scripts live)
VIRTMCU_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Target directory to lint (defaults to VIRTMCU_ROOT)
# Usage: ./run-lint.sh [target_dir]
TARGET_DIR="$(cd "${1:-$VIRTMCU_ROOT}" && pwd)"

echo "════════════════════════════════════════════════════"
echo "  Running All Lints"
echo "  Target: $TARGET_DIR"
echo "  Scripts: $VIRTMCU_ROOT"
echo "════════════════════════════════════════════════════"

# Detect if we are inside docker
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || [ -f /run/.containerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

# Set up PYTHONPATH to include VIRTMCU_ROOT for utility imports
export PYTHONPATH="${VIRTMCU_ROOT}:${PYTHONPATH:-.}"

# Helper to run commands (with or without uv)
run_cmd() {
    if [ "$INSIDE_DOCKER" = "true" ]; then
        "$@"
    else
        uv run --no-project "$@"
    fi
}

run_uvx() {
    if [ "$INSIDE_DOCKER" = "true" ]; then
        # Assume tools are installed in docker
        "$@"
    else
        uvx "$@"
    fi
}

# If we are linting something other than VIRTMCU_ROOT, we apply rules more broadly
FORCE_ALL_FLAG=""
if [ "$TARGET_DIR" != "$VIRTMCU_ROOT" ]; then
    FORCE_ALL_FLAG="--force-all"
fi

cd "$TARGET_DIR"

echo "==> Checking version synchronization..."
run_cmd python3 "$VIRTMCU_ROOT/scripts/check-versions.py" --root .

echo "==> Verifying FFI layouts..."
run_cmd python3 "$VIRTMCU_ROOT/scripts/check-ffi.py"

echo "==> Verifying plugin exports..."
run_cmd python3 "$VIRTMCU_ROOT/scripts/verify-exports.py"

echo "==> Python lints..."
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/dependency_pinning.py" .
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/beyonce_rule.py"
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/third_party_modifications.py"
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/python_banned_patterns.py" . $FORCE_ALL_FLAG
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/simulation_usage.py" .
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/environment_agnosticism.py" .
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/rust_static_state.py" .
run_cmd ruff check .
echo "✓ ruff passed."

echo "==> Python type checking (mypy)..."
# Only run mypy if the target has typical python dirs
MYPY_DIRS=""
for d in tools tests patches; do
    if [ -d "$d" ]; then MYPY_DIRS="$MYPY_DIRS $d"; fi
done
if [ -n "$MYPY_DIRS" ]; then
    run_cmd mypy $MYPY_DIRS
    echo "✓ mypy passed."
fi

echo "==> Rust lints..."
if [ -f "Cargo.toml" ]; then
    # Workspace version sync
    cargo metadata --no-deps --format-version 1 | \
        python3 -c "import sys,json; m=json.load(sys.stdin); vs=set(p['version'] for p in m['packages']); assert len(vs)==1, f'version drift: {vs}'"
    echo "✓ Cargo workspace versions aligned."

    cargo fmt --all --check
    cargo machete

    run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/rust_banned_patterns.py" . $FORCE_ALL_FLAG
    run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/rust_static_state.py" .
    run_cmd python3 "$VIRTMCU_ROOT/scripts/check-stale-so.py"
    run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/rust_safe_serialization.py" .
    run_cmd python3 "$VIRTMCU_ROOT/scripts/check-qom-alignment.py"
    run_cmd python3 "$VIRTMCU_ROOT/scripts/check-cargo-meson-lib-alignment.py"

    cargo clippy --workspace -- -D warnings
    echo "✓ clippy passed."
fi

echo "==> Shell lints..."
run_cmd python3 "$VIRTMCU_ROOT/scripts/lints/shell_lints.py" .

echo "==> Docker lints..."
if [ -f "docker/Dockerfile" ]; then
    if command -v hadolint >/dev/null 2>&1; then
        hadolint docker/Dockerfile
        echo "✓ hadolint passed."
    else
        echo "⚠ Skipping hadolint (not installed)"
    fi
fi

echo "==> Action lints..."
if [ -d ".github/workflows" ]; then
    if command -v actionlint >/dev/null 2>&1; then
        actionlint
        echo "✓ actionlint passed."
    else
        echo "⚠ Skipping actionlint (not installed)"
    fi
fi

echo "==> YAML lints..."
while IFS= read -r -d '' file; do
    run_uvx yamllint --strict -d "{extends: relaxed, rules: {line-length: disable}}" "$file"
done < <(find . -type f \( -name "*.yml" -o -name "*.yaml" \) -not -path "*/third_party/*" -not -path "*/build/*" -not -path "*/target/*" -not -path "*/.claude/*" -not -path "*/.cargo-cache/*" -not -path "*/schema/node_modules/*" -print0)
echo "✓ yamllint passed."

echo "==> C lints..."
C_DIRS=""
for d in hw tools tests; do
    if [ -d "$d" ]; then C_DIRS="$C_DIRS $d"; fi
done

if [ -n "$C_DIRS" ] && command -v clang-format >/dev/null 2>&1; then
    find $C_DIRS -type f \( -name "*.c" -o -name "*.h" -o -name "*.cpp" -o -name "*.cc" -o -name "*.hpp" \) \
        -not -path "*/rust/*" \
        -not -path "*/remote-port/*" \
        -not -path "*/third_party/*" \
        -not -path "*/build/*" \
        -not -path "*/target/*" \
        -print0 | xargs -0 clang-format -Werror --dry-run
    echo "✓ clang-format passed."
fi

if [ -d "hw/misc" ] && command -v cppcheck >/dev/null 2>&1; then
    cppcheck --error-exitcode=1 --enable=warning,style,performance,portability --quiet --std=c11 \
        --suppress=unusedFunction \
        --suppress=arithOperationsOnVoidPointer \
        --suppress=normalCheckLevelMaxBranches \
        --suppress=uninitvar --suppress=legacyUninitvar \
        --suppress=knownConditionTrueFalse \
        --suppress=identicalInnerCondition \
        --suppress=dangerousTypeCast \
        --suppress=constVariablePointer --suppress=constParameterPointer \
        --suppress=redundantAssignment --suppress=noExplicitConstructor \
        --suppress=constParameterCallback --suppress=unusedVariable \
        -DDEFINE_TYPES= -DHWADDR_PRIx=\"lx\" -DPRIx64=\"llx\" \
        hw/misc/
    echo "✓ cppcheck passed."
fi

echo "==> Spelling lints..."
run_uvx codespell --skip="./third_party/*,**/build/*,**/target/*,**/target-*/*,./.git/*,./.claude/*,Cargo.lock,uv.lock,./patches/*,./coverage_report/*,./test-results/*,./.cargo-cache/*,./temp/*,./schema/node_modules/*,./schema/package-lock.json,mermaid.min.js,mermaid-init.js" \
    --ignore-words-list="virtmcu,zenoh,qemu,qmp,riscv,TE" .
echo "✓ codespell passed."

echo "==> Audit lints..."
if [ -f "Cargo.lock" ]; then
    # Use a workspace-local advisory database
    ADVISORY_DB="$TARGET_DIR/.cargo-cache/advisory-db"
    mkdir -p "$ADVISORY_DB"

    if command -v cargo-audit >/dev/null 2>&1; then
        cargo audit --db "$ADVISORY_DB" --stale --ignore RUSTSEC-2026-0041 --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2024-0436 --ignore RUSTSEC-2025-0134 -f Cargo.lock
    fi

    if command -v cargo-deny >/dev/null 2>&1; then
        cargo deny check
        echo "✓ cargo deny passed."
    fi
fi

if [ -f "hw/meson.build" ]; then
    echo "==> Meson lints..."
    run_uvx meson format -q hw/meson.build
    echo "✓ meson format passed."
fi

echo "════════════════════════════════════════════════════"
echo "  All Lints Passed!"
echo "════════════════════════════════════════════════════"
