#!/usr/bin/env bash
# tests/fixtures/guest_apps/dynamic_plugin/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/system/test_device_realization.py
