#!/usr/bin/env bash
# tests/fixtures/guest_apps/phase6/smoke_test.sh — Phase 6 smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/test_phase6.py
