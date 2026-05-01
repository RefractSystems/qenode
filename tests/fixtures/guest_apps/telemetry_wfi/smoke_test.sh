#!/usr/bin/env bash
# tests/fixtures/guest_apps/telemetry_wfi/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/system/test_telemetry.py
