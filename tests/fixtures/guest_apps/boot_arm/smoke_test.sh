#!/usr/bin/env bash
# tests/fixtures/guest_apps/boot_arm/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/core/test_boot_arm.py
