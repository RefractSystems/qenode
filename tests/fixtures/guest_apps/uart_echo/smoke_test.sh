#!/usr/bin/env bash
# tests/fixtures/guest_apps/uart_echo/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tests/integration/peripherals/test_uart_echo.py
