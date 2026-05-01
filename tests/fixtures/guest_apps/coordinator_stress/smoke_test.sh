#!/usr/bin/env bash
# tests/fixtures/guest_apps/coordinator_stress/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/system/test_coordinator.py
