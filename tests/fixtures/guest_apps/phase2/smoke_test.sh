#!/usr/bin/env bash
# tests/fixtures/guest_apps/phase2/smoke_test.sh — Phase 2 smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/test_phase2.py
