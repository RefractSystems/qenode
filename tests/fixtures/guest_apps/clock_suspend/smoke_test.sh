#!/usr/bin/env bash
# tests/fixtures/guest_apps/clock_suspend/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/system/test_clock_suspend.py
