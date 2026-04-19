#!/usr/bin/env bash
# test/phase12/telemetry_throughput_test.sh — Phase 12.8 Telemetry Throughput Benchmark
#
# Runs the zenoh-telemetry plugin under IRQ-storm load and verifies that the
# host-side event throughput reaches ≥ 100,000 events/second without stalling
# the vCPU (no timeout errors from QEMU).
#
# This script is NOT part of the default smoke_test.sh sweep because it
# requires QEMU to be built with zenoh-telemetry and is slow (~10 s).
# Run explicitly with:
#   bash test/phase12/telemetry_throughput_test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "============================================================"
echo "Phase 12.8 — Telemetry Throughput Benchmark"
echo "============================================================"

# Build test artifacts if needed.
make -C "$SCRIPT_DIR" test_irq_storm.elf test_telemetry.dtb

# Run the Python benchmark harness.
PYTHONPATH="$WORKSPACE_DIR" python3 "$SCRIPT_DIR/telemetry_bench.py"
