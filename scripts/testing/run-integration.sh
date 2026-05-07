#!/usr/bin/env bash
# ==============================================================================
# scripts/testing/run-integration.sh - Unified Integration Tier Runner
#
# This script is the SINGLE SOURCE OF TRUTH for running integration tests.
# It can be used directly or invoked by downstream projects submoduling virtmcu.
# ==============================================================================
set -euo pipefail

DOMAIN="${1:-all}"

# SOTA: Source common helpers (sets WORKSPACE_DIR, SCRIPTS_DIR, PYTHONPATH, etc.)
# shellcheck source=scripts/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"

# Ensure consistent ASan/UBSan behavior if enabled
if [ "${VIRTMCU_USE_ASAN:-0}" = "1" ]; then
    export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=0,halt_on_error=1,detect_stack_use_after_return=1}"
    export UBSAN_OPTIONS="${UBSAN_OPTIONS:-halt_on_error=1:print_stacktrace=1}"
    echo "==> ASan/UBSan enabled. Options: $ASAN_OPTIONS"
fi

# Detect if we are inside the builder container
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

# Ensure we have the environment set up (only if inside docker)
if [ "$INSIDE_DOCKER" = "true" ]; then
    # Helper for sudo if not root
    SUDO=""
    if [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    fi

    # Ensure Python dependencies are synced in the container
    mkdir -p target
    PYPROJECT_HASH=$(sha256sum pyproject.toml uv.lock | sha256sum | cut -c1-12)
    SYNC_MARKER="target/.ci_marker_uv_synced_${PYPROJECT_HASH}"

    if [ ! -f "$SYNC_MARKER" ]; then
        echo "==> Syncing Python dependencies inside container (hash: ${PYPROJECT_HASH})..."
        # Clean up old markers
        rm -f target/.ci_marker_uv_synced_*
        $SUDO uv pip install --link-mode=copy --system --break-system-packages . >/dev/null
        touch "$SYNC_MARKER"
        echo "✓ Python dependencies synced."
    fi

    if [ ! -f target/.ci_marker_artifacts_built ]; then
        echo "==> Building test artifacts..."
        make build-test-artifacts >/dev/null
        touch target/.ci_marker_artifacts_built
    fi
fi

run_domain() {
    local d=$1
    echo "════════════════════════════════════════════════════"
    echo "  Running Integration Domain: $d"
    echo "════════════════════════════════════════════════════"

    case "$d" in
        boot_arm)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_boot_arm.py -v --tb=short
            ;;
        yaml_boot)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_repl_boot.py -v --tb=short
            ;;
        yaml_boot_advanced)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_yaml_boot.py -v --tb=short
            ;;
        qmp_failures)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/tooling/test_qmp_failures.py -v --tb=short
            ;;
        irq_stress)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/infrastructure/test_architecture_stress.py -v --tb=short
            ;;
        coordinator_stress)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/infrastructure/test_coordinator_stress.py -v --tb=short
            ;;
        clock_suspend)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/infrastructure/test_clock_suspend.py -v --tb=short
            ;;
        ftrt_timing)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/infrastructure/test_ftrt_timing.py -v --tb=short
            ;;
        cyber_bridge)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/tooling/test_cyber_bridge.py -v --tb=short
            ;;
        riscv_complex)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_boot_riscv.py -v --tb=short
            ;;
        riscv_interrupts)
            # Fixture migrated to core RISC-V boot test
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_boot_riscv.py -v --tb=short
            ;;
        telemetry_wfi)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/peripherals/test_telemetry.py -v --tb=short
            ;;
        plugin_multiplexing)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_plugin_multiplexing.py -v --tb=short
            ;;
        priority_routing)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/infrastructure/test_clock_priority.py -v --tb=short
            ;;
        complex_board)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/core/test_complex_board.py -v --tb=short
            ;;
        coverage_gap)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/tooling/test_coverage_gap.py -v --tb=short
            ;;
        perf_bench)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/infrastructure/test_jitter_proxy.py -v --tb=short
            ;;
        bql_stress)
            bash tests/fixtures/guest_apps/bql_stress/bql_stress_test.sh
            bash tests/fixtures/guest_apps/bql_stress/netdev_flood_test.sh
            bash tests/fixtures/guest_apps/bql_stress/qom_registration_test.sh
            ;;
        flexray_bridge)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/peripherals/test_flexray.py -v --tb=short
            ;;
        spi_bridge)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/peripherals/test_spi.py -v --tb=short
            ;;
        mac_parsing)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/peripherals/test_mac_parsing.py tests/integration/simulation/peripherals/test_spi_multibus.py -v --tb=short
            ;;
        lin_bridge)
            pytest -n "${PYTEST_WORKERS:-auto}" tests/integration/simulation/peripherals/test_lin.py tests/integration/simulation/peripherals/test_lin_multi_node.py tests/integration/simulation/peripherals/test_lin_stress.py -v --tb=short
            ;;
        qmp)
            make -C tests/fixtures/guest_apps/boot_arm && make -C tests/fixtures/guest_apps/uart_echo && pytest -n "${PYTEST_WORKERS:-auto}" tools/testing/test_qmp.py tests/integration/tooling/ -v --tb=short
            ;;
        *)
            echo "ERROR: Unknown domain '$d'"
            exit 1
            ;;
    esac
}

if [ "$DOMAIN" = "all" ]; then
    echo "════════════════════════════════════════════════════"
    echo "  Running All Integration Domains (xargs parallel)"
    echo "════════════════════════════════════════════════════"
    
    # Ensure a clean slate before starting
    bash scripts/cleanup-sim.sh --quiet
    
    # The authoritative list of domains that MUST pass
    DOMAINS="boot_arm yaml_boot yaml_boot_advanced qmp_failures irq_stress coordinator_stress clock_suspend ftrt_timing cyber_bridge riscv_complex riscv_interrupts telemetry_wfi plugin_multiplexing priority_routing complex_board coverage_gap perf_bench bql_stress flexray_bridge spi_bridge mac_parsing lin_bridge qmp"
    
    # Use xargs to run domains in parallel.
    # Export the function so xargs can use it via bash -c
    # -P 0 uses all available CPUs for maximum parallelism.
    export -f run_domain
    export PYTEST_WORKERS=${PYTEST_WORKERS:-auto}
    export ASAN_OPTIONS
    export UBSAN_OPTIONS
    export PYTHONPATH

    echo "$DOMAINS" | tr ' ' '\n' | xargs -P 0 -I {} bash -c 'run_domain "{}"' || {
        echo "❌ One or more domains failed."
        exit 1
    }
    
    echo "✅ All domains passed."
else
    # Ensure a clean slate before starting a single domain
    bash scripts/cleanup-sim.sh --quiet
    run_domain "$DOMAIN"
fi
