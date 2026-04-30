#!/usr/bin/env bash
# tests/fixtures/guest_apps/phase1/smoke_test.sh — Phase 1 smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/test_phase1.py
