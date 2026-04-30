#!/usr/bin/env bash
# tests/fixtures/guest_apps/phase12/smoke_test.sh — Phase 12 smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/test_phase12.py
