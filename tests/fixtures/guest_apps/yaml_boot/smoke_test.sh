#!/usr/bin/env bash
# tests/fixtures/guest_apps/yaml_boot/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/core/test_repl_boot.py
