#!/bin/bash
set -euo pipefail

# Change to the script's directory, then up to the workspace root
cd "$(dirname "$0")/.."

echo "⚙️  1. Compiling TypeSpec..."
cd schema
npx tsp compile world/main.tsp --output-dir ./dist
cp dist/@typespec/json-schema/virtmcu_world.schema.json world_schema.json
cd ..

echo "🔧 2. Fixing JSON Schema References..."
python3 scripts/fix_json_schema.py

echo "🐍 3. Generating Python Models (Pydantic v2)..."
uv run --with datamodel-code-generator datamodel-codegen \
    --input schema/world_schema.json \
    --output tools/testing/virtmcu_test_suite/generated.py \
    --input-file-type jsonschema \
    --output-model-type pydantic_v2.BaseModel \
    --disable-timestamp \
    --allow-extra-fields

echo "🖌️  3.5. Formatting Python Models..."
uv run --active ruff format tools/testing/virtmcu_test_suite/generated.py

echo "🦀 4. Generating Rust Models (Serde)..."
cd schema/rust_gen
cargo run
cd ../..
rustfmt tools/deterministic_coordinator/src/generated/topology.rs || true

echo "🖌️  4.5. Formatting Rust Models..."
rustfmt --edition 2021 tools/deterministic_coordinator/src/generated/topology.rs

echo "✅ Code generation pipeline completed successfully!"
