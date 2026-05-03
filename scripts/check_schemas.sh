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

# Run generation in temp
pushd "$TEMP_DIR" > /dev/null
./scripts/generate_schemas.sh > /dev/null 2>&1
popd > /dev/null

# Compare generated files
if ! cmp -s schema/world_schema.json "$TEMP_DIR/schema/world_schema.json"; then
    echo "❌ Error: world_schema.json is out of date. Please run ./scripts/generate_schemas.sh"
    exit 1
fi

if ! cmp -s tools/testing/virtmcu_test_suite/generated.py "$TEMP_DIR/tools/testing/virtmcu_test_suite/generated.py"; then
    echo "❌ Error: generated.py is out of date. Please run ./scripts/generate_schemas.sh"
    exit 1
fi

if ! cmp -s tools/deterministic_coordinator/src/generated/topology.rs "$TEMP_DIR/tools/deterministic_coordinator/src/generated/topology.rs"; then
    echo "❌ Error: topology.rs is out of date. Please run ./scripts/generate_schemas.sh"
    exit 1
fi

echo "✅ Generated schema artifacts are perfectly synchronized with the TypeSpec source."
