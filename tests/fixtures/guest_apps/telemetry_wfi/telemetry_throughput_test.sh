#!/usr/bin/env bash
# tests/fixtures/guest_apps/telemetry_wfi/telemetry_throughput_test.sh — Telemetry Throughput Benchmark
#
# Runs the telemetry plugin under IRQ-storm load and verifies that the
# host-side event throughput reaches ≥ 100,000 events/second without stalling
# the vCPU (no timeout errors from QEMU).
#
# This script is NOT part of the default smoke_test.sh sweep because it
# requires QEMU to be built with telemetry and is slow (~10 s).
# Run explicitly with:
#   bash tests/fixtures/guest_apps/telemetry_wfi/telemetry_throughput_test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Find workspace root (robustly)
_search_dir="$SCRIPT_DIR"
while [[ "$_search_dir" != "/" ]]; do
    if [[ -f "$_search_dir/scripts/common.sh" ]]; then
        source "$_search_dir/scripts/common.sh"
        break
    fi
    _search_dir=$(dirname "$_search_dir")
done

if [[ -z "${WORKSPACE_DIR:-}" ]]; then
    echo "ERROR: Could not find scripts/common.sh" >&2
    exit 1
fi

echo "============================================================"
echo "— Telemetry Throughput Benchmark"
echo "============================================================"

# Build test artifacts if needed.
make -C "$SCRIPT_DIR" test_irq_storm.elf test_telemetry.dtb

# Run the Python benchmark harness.
PYTHONPATH="$WORKSPACE_DIR" python3 "$SCRIPT_DIR/telemetry_bench.py"
