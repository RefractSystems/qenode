#!/usr/bin/env bash
# tests/fixtures/guest_apps/qmp_failures/smoke_test.sh — smoke test (Modernized to pytest)
set -euo pipefail
pytest tools/testing/test_qmp.py tests/unit/test_qmp_bridge.py tests/integration/system/test_qemu_library_pytest.py tests/integration/system/test_qmp_failures.py -v --tb=short
