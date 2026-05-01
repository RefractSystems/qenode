#!/usr/bin/env bash
# tests/fixtures/guest_apps/cyber_bridge/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/system/test_cyber_bridge.py
