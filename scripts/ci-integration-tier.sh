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
    # Ensure system dependencies are present for specific domains
    case "$DOMAIN" in
        irq_stress|ftrt_timing|riscv_interrupts|all)
            if ! dpkg -l | grep libsystemc-dev >/dev/null; then
                echo "==> Installing SystemC dependencies..."
                apt-get update -qq && apt-get install -y -qq --no-install-recommends libsystemc-dev >/dev/null
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
        uv pip install --link-mode=copy --system --break-system-packages . >/dev/null
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
            make -C tests/fixtures/guest_apps/boot_arm && bash tests/fixtures/guest_apps/boot_arm/smoke_test.sh
            ;;
        yaml_boot)
            bash tests/fixtures/guest_apps/yaml_boot/smoke_test.sh
            ;;
        yaml_boot_advanced)
            make -C tests/fixtures/guest_apps/boot_arm && bash tests/fixtures/guest_apps/yaml_boot_advanced/smoke_test.sh
            ;;
        qmp_failures)
            make -C tests/fixtures/guest_apps/boot_arm && bash tests/fixtures/guest_apps/qmp_failures/smoke_test.sh
            ;;
        irq_stress)
            bash tests/fixtures/guest_apps/irq_stress/smoke_test.sh
            ;;
        coordinator_stress)
            bash tests/fixtures/guest_apps/coordinator_stress/smoke_test.sh
            ;;
        clock_suspend)
            bash tests/fixtures/guest_apps/clock_suspend/smoke_test.sh
            ;;
        ftrt_timing)
            make -C tests/fixtures/guest_apps/boot_arm && bash tests/fixtures/guest_apps/ftrt_timing/smoke_test.sh
            ;;
        cyber_bridge)
            make -C tests/fixtures/guest_apps/boot_arm && bash tests/fixtures/guest_apps/cyber_bridge/smoke_test.sh
            ;;
        riscv_complex)
            make -C tests/fixtures/guest_apps/boot_riscv && bash tests/fixtures/guest_apps/riscv_complex/smoke_test.sh
            ;;
        riscv_interrupts)
            cmake -S tools/systemc_adapter -B tools/systemc_adapter/build -DCMAKE_BUILD_TYPE=Release >/dev/null
            make -C tools/systemc_adapter/build rp_adapter >/dev/null
            bash tests/fixtures/guest_apps/riscv_interrupts/smoke_test.sh
            ;;
        telemetry_wfi)
            make -C tests/fixtures/guest_apps/boot_arm && make -C tests/fixtures/guest_apps/telemetry_wfi && bash tests/fixtures/guest_apps/telemetry_wfi/smoke_test.sh
            ;;
        priority_routing)
            bash tests/fixtures/guest_apps/priority_routing/smoke_test.sh
            ;;
        complex_board)
            bash tests/fixtures/guest_apps/complex_board/smoke_test.sh
            ;;
        coverage_gap)
            bash tests/fixtures/guest_apps/coverage_gap/smoke_test.sh
            ;;
        perf_bench)
            make -C tests/fixtures/guest_apps/boot_arm && make -C tests/fixtures/guest_apps/perf_bench && bash tests/fixtures/guest_apps/perf_bench/smoke_test.sh
            ;;
        bql_stress)
            bash tests/fixtures/guest_apps/bql_stress/bql_stress_test.sh
            bash tests/fixtures/guest_apps/bql_stress/netdev_flood_test.sh
            bash tests/fixtures/guest_apps/bql_stress/qom_registration_test.sh
            ;;
        flexray_bridge)
            PYTHONPATH=$(pwd) pytest tests/integration/peripherals/test_flexray.py -v --tb=short
            ;;
        spi_bridge)
            pytest tests/integration/peripherals/test_spi.py -v --tb=short
            ;;
        mac_parsing)
            pytest tests/integration/peripherals/test_mac_parsing.py tests/integration/peripherals/test_spi_multibus.py -v --tb=short
            ;;
        lin_bridge)
            pytest tests/integration/peripherals/test_lin.py tests/integration/peripherals/test_lin_multi_node.py tests/integration/peripherals/test_lin_stress.py -v --tb=short
            ;;
        qmp)
            make -C tests/fixtures/guest_apps/boot_arm && make -C tests/fixtures/guest_apps/uart_echo && pytest tools/testing/test_qmp.py -v --tb=short
            ;;
        robot)
            make -C tests/fixtures/guest_apps/boot_arm && make -C tests/fixtures/guest_apps/uart_echo && robot --outputdir test-results/robot --xunit test-results/robot.xml tests/integration/peripherals/test_qmp_keywords.robot tests/integration/peripherals/test_uart_interactive.robot
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
    for d in boot_arm yaml_boot yaml_boot_advanced qmp_failures irq_stress coordinator_stress clock_suspend ftrt_timing cyber_bridge riscv_complex riscv_interrupts telemetry_wfi priority_routing complex_board coverage_gap perf_bench bql_stress flexray_bridge spi_bridge mac_parsing lin_bridge qmp robot; do
        run_domain "$d"
    done
else
    run_domain "$DOMAIN"
fi
