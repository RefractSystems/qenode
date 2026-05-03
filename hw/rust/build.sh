#!/usr/bin/env bash
set -euo pipefail

# SOTA Error visibility
trap 'echo "ERROR: ${BASH_SOURCE[0]} failed at line $LINENO" >&2' ERR

# $1: rust source dir (hw/rust)
# $2: target dir (hw/target or similar)
# $3: out dir (where .a files should go)
# rest: pairs of "package-name:libname.a"

RUST_DIR="${1:-}"
TARGET_DIR="${2:-}"
OUT_DIR="${3:-}"
shift 3

cd "$RUST_DIR"

if command -v lld >/dev/null 2>&1; then
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-fuse-ld=lld"
fi

if [ "${VIRTMCU_USE_ASAN:-}" = "1" ]; then
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-fsanitize=address -C link-arg=-fsanitize=undefined"
elif [ "${VIRTMCU_USE_TSAN:-}" = "1" ]; then
    export RUSTC_BOOTSTRAP=1
    export RUSTFLAGS="${RUSTFLAGS:-} -Z sanitizer=thread"
fi

echo "Building Rust workspace in $RUST_DIR with target-dir $TARGET_DIR"

# Detect if TARGET_DIR is on a suspect mount (virtiofs, fakeowner) common in Docker on macOS/Windows
# These filesystems have known issues with mmap and file locking that can cause
# "Bus error" (SIG7) and Cargo fingerprint corruption.
FS_TYPE="$(df -T "$TARGET_DIR" 2>/dev/null | awk 'NR==2 {print $2}' || true)"
if [ -z "$FS_TYPE" ]; then
    FS_TYPE="$(df -T "$(dirname "$TARGET_DIR")" 2>/dev/null | awk 'NR==2 {print $2}' || true)"
fi

if [ "$FS_TYPE" = "virtiofs" ] || [ "$FS_TYPE" = "fakeowner" ] || [ "$FS_TYPE" = "9p" ]; then
    SAFE_TARGET_DIR="/tmp/virtmcu-rust-target-$(id -u)"
    echo "WARNING: $TARGET_DIR is on a $FS_TYPE mount. Redirecting Cargo target-dir to $SAFE_TARGET_DIR to avoid Bus errors."
    TARGET_DIR="$SAFE_TARGET_DIR"
fi

mkdir -p "$TARGET_DIR"

# Disconnect from Ninja's jobserver to prevent E0463 race conditions during
# parallel builds. Only MAKEFLAGS carries the jobserver token; unsetting it is
# sufficient. Cargo then manages its own thread pool independently.
unset MAKEFLAGS
cargo build --release --workspace --target-dir "$TARGET_DIR"

for pair in "$@"; do
    _pkg="${pair%%:*}"
    lib="${pair#*:}"
    echo "Copying $TARGET_DIR/release/$lib to $OUT_DIR/$lib"
    cp "$TARGET_DIR/release/$lib" "$OUT_DIR/$lib"
done

echo "Listing outputs in $OUT_DIR:"
ls -lh "$OUT_DIR"/*.a
