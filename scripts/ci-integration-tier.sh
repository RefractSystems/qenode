#!/usr/bin/env bash
# ==============================================================================
# scripts/ci-integration-tier.sh - Unified CI Integration Tier Runner
#
# This script is the SINGLE SOURCE OF TRUTH for running CI integration tests.
# It is used by .github/workflows/ci-pr.yml, ci-main.yml, and the local Makefile.
# Domain names and ordering are defined in .github/smoke-domains.json.
# ==============================================================================
set -euo pipefail

DOMAIN="${1:-all}"

# Detect if we are inside the builder container
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

# Ensure we have the environment set up (only if inside docker)
if [ "$INSIDE_DOCKER" = "true" ]; then
    export PYTHONPATH="${PYTHONPATH:-}:/workspace"

    # Helper for sudo if not root
    SUDO=""
    if [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    fi

    # Ensure system dependencies are present for specific domains
    case "$DOMAIN" in
        irq_stress|ftrt_timing|riscv_interrupts|all)
            if ! dpkg -l | grep libsystemc-dev >/dev/null; then
                echo "==> Installing SystemC dependencies..."
                $SUDO apt-get update -qq && $SUDO apt-get install -y -qq --no-install-recommends libsystemc-dev >/dev/null
            fi
            ;;
    esac
    
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

    # Ensure a clean slate before each domain
    bash scripts/cleanup-sim.sh --quiet

    case "$d" in
        boot_arm)
            pytest tests/integration/simulation/core/test_boot_arm.py -v --tb=short
            ;;
        yaml_boot)
            pytest tests/integration/simulation/core/test_repl_boot.py -v --tb=short
            ;;
        yaml_boot_advanced)
            pytest tests/integration/simulation/core/test_yaml_boot.py -v --tb=short
            ;;
        qmp_failures)
            pytest tests/integration/tooling/test_qmp_failures.py -v --tb=short
            ;;
        irq_stress)
            pytest tests/integration/infrastructure/test_architecture_stress.py -v --tb=short
            ;;
        coordinator_stress)
            pytest tests/integration/infrastructure/test_coordinator_stress.py -v --tb=short
            ;;
        clock_suspend)
            pytest tests/integration/infrastructure/test_clock_suspend.py -v --tb=short
            ;;
        ftrt_timing)
            pytest tests/integration/infrastructure/test_ftrt_timing.py -v --tb=short
            ;;
        cyber_bridge)
            pytest tests/integration/tooling/test_cyber_bridge.py -v --tb=short
            ;;
        riscv_complex)
            pytest tests/integration/simulation/core/test_boot_riscv.py -v --tb=short
            ;;
        riscv_interrupts)
            # Fixture migrated to core RISC-V boot test
            pytest tests/integration/simulation/core/test_boot_riscv.py -v --tb=short
            ;;
        telemetry_wfi)
            pytest tests/integration/simulation/peripherals/test_telemetry.py -v --tb=short
            ;;
        priority_routing)
            pytest tests/integration/infrastructure/test_clock_priority.py -v --tb=short
            ;;
        complex_board)
            pytest tests/integration/simulation/core/test_complex_board.py -v --tb=short
            ;;
        coverage_gap)
            pytest tests/integration/tooling/test_coverage_gap.py -v --tb=short
            ;;
        perf_bench)
            pytest tests/integration/infrastructure/test_jitter_proxy.py -v --tb=short
            ;;
        bql_stress)
            bash tests/fixtures/guest_apps/bql_stress/bql_stress_test.sh
            bash tests/fixtures/guest_apps/bql_stress/netdev_flood_test.sh
            bash tests/fixtures/guest_apps/bql_stress/qom_registration_test.sh
            ;;
        flexray_bridge)
            PYTHONPATH=$(pwd) pytest tests/integration/simulation/peripherals/test_flexray.py -v --tb=short
            ;;
        spi_bridge)
            pytest tests/integration/simulation/peripherals/test_spi.py -v --tb=short
            ;;
        mac_parsing)
            pytest tests/integration/simulation/peripherals/test_mac_parsing.py tests/integration/simulation/peripherals/test_spi_multibus.py -v --tb=short
            ;;
        lin_bridge)
            pytest tests/integration/simulation/peripherals/test_lin.py tests/integration/simulation/peripherals/test_lin_multi_node.py tests/integration/simulation/peripherals/test_lin_stress.py -v --tb=short
            ;;
        qmp)
            make -C tests/fixtures/guest_apps/boot_arm && make -C tests/fixtures/guest_apps/uart_echo && pytest tools/testing/test_qmp.py tests/integration/tooling/ -v --tb=short
            ;;
        *)
            echo "ERROR: Unknown domain '$d'"
            exit 1
            ;;
    esac
}

if [ "$DOMAIN" = "all" ]; then
    # The authoritative list of domains that MUST pass
    # (Matches the matrix in .github/smoke-domains.json)
    for d in boot_arm yaml_boot yaml_boot_advanced qmp_failures irq_stress coordinator_stress clock_suspend ftrt_timing cyber_bridge riscv_complex riscv_interrupts telemetry_wfi priority_routing complex_board coverage_gap perf_bench bql_stress flexray_bridge spi_bridge mac_parsing lin_bridge qmp; do
        run_domain "$d"
    done
else
    run_domain "$DOMAIN"
fi
