#!/bin/bash
set -euo pipefail

# Change to the script's directory, then up to the workspace root
cd "$(dirname "$0")/.."

echo "==> Verifying schema generation is up-to-date..."

# Create a temporary directory
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# Copy the schema files
cp -r schema "$TEMP_DIR/"
cp -r scripts "$TEMP_DIR/"
cp -r tools "$TEMP_DIR/"
cp pyproject.toml "$TEMP_DIR/"
cp Cargo.toml "$TEMP_DIR/"
mkdir -p "$TEMP_DIR/hw/rust/common/virtmcu-api"
cp -r hw/rust/common/virtmcu-api/src "$TEMP_DIR/hw/rust/common/virtmcu-api/"
mkdir -p "$TEMP_DIR/hw/misc"
cp hw/misc/*.fbs "$TEMP_DIR/hw/misc/"
cp -r hw/rust/rustfmt.toml "$TEMP_DIR/hw/rust/" 2>/dev/null || true

# Run generation in temp
pushd "$TEMP_DIR" > /dev/null
./scripts/generate_schemas.sh
popd > /dev/null

# Compare generated files
if ! cmp -s schema/world_schema.json "$TEMP_DIR/schema/world_schema.json"; then
    echo "❌ Error: world_schema.json is out of date. Please run ./scripts/generate_schemas.sh"
    exit 1
fi

if ! cmp -s tools/testing/virtmcu_test_suite/generated.py "$TEMP_DIR/tools/testing/virtmcu_test_suite/generated.py"; then
    echo "❌ Error: generated.py is out of date. Please run ./scripts/generate_schemas.sh"
    diff tools/testing/virtmcu_test_suite/generated.py "$TEMP_DIR/tools/testing/virtmcu_test_suite/generated.py" | head -n 20
    exit 1
fi

if ! cmp -s tools/deterministic_coordinator/src/generated/topology.rs "$TEMP_DIR/tools/deterministic_coordinator/src/generated/topology.rs"; then
    echo "❌ Error: topology.rs is out of date. Please run ./scripts/generate_schemas.sh"
    exit 1
fi

echo "🥞 4. Verifying FlatBuffers Python bindings..."
# Check a few key files to ensure they are synchronized
FB_FILES=(
    "tools/virtmcu/core/CoordMessage.py"
    "tools/virtmcu/rf802154/Rf802154Header.py"
    "tools/virtmcu/can/CanFdFrame.py"
)

for f in "${FB_FILES[@]}"; do
    if ! cmp -s "$f" "$TEMP_DIR/$f"; then
        echo "❌ Error: $f is out of date. Please run ./scripts/generate_schemas.sh"
        exit 1
    fi
done

echo "✅ Generated schema artifacts are perfectly synchronized with the TypeSpec and FlatBuffers sources."
