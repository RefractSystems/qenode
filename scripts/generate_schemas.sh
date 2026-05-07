#!/bin/bash
set -euo pipefail

# Change to the script's directory, then up to the workspace root
cd "$(dirname "$0")/.."

echo "⚙️  1. Compiling TypeSpec..."
cd schema
npx tsp compile world/main.tsp --output-dir ./dist
cp dist/@typespec/json-schema/virtmcu_world.schema.json world_schema.json
cd ..

echo "🥞 1.5. Regenerating FlatBuffers Python bindings..."
flatc --python --python-typing -o generated/ hw/rust/common/virtmcu-api/src/core.fbs hw/rust/common/virtmcu-api/src/rf802154.fbs hw/rust/common/virtmcu-api/src/can.fbs hw/rust/common/virtmcu-api/src/lin.fbs hw/rust/common/virtmcu-api/src/flexray.fbs hw/misc/telemetry.fbs

echo "🔧 2. Fixing JSON Schema References..."
python3 scripts/fix_json_schema.py

# Detect if we are inside docker
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || [ -f /run/.containerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

# Use --no-project to avoid creating virtual environments in the workspace
UV_OPTS=""
if [ "$INSIDE_DOCKER" = "true" ]; then
    UV_OPTS="--no-project"
fi

echo "🐍 3. Generating Python Models (Pydantic v2)..."
uv run $UV_OPTS --with datamodel-code-generator datamodel-codegen \
    --input schema/world_schema.json \
    --output generated/world_schema.py \
    --input-file-type jsonschema \
    --output-model-type pydantic_v2.BaseModel \
    --disable-timestamp \
    --allow-extra-fields

echo "🖌️  3.5. Formatting Python Models..."
uv run $UV_OPTS ruff format generated/world_schema.py

echo "🦀 4. Generating Rust Models (Serde)..."
cd schema/rust_gen
cargo run
cd ../..
rustfmt tools/deterministic_coordinator/src/generated/topology.rs || true

echo "🖌️  4.5. Formatting Rust Models..."
rustfmt --edition 2021 tools/deterministic_coordinator/src/generated/topology.rs

echo "✅ Code generation pipeline completed successfully!"
