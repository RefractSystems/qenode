#!/usr/bin/env bash
# ==============================================================================
# scripts/ci-phase.sh - Unified CI Phase Runner
#
# This script is the SINGLE SOURCE OF TRUTH for running CI phases.
# It is used by .github/workflows/ci-pr.yml, ci-main.yml, and the local Makefile.
# Phase names and ordering are defined in .github/smoke-phases.json.
# ==============================================================================
set -euo pipefail

PHASE="${1:-all}"

# Detect if we are inside the builder container
INSIDE_DOCKER=false
if [ -f /.dockerenv ] || grep -q "docker" /proc/1/cgroup 2>/dev/null; then
    INSIDE_DOCKER=true
fi

# Ensure we have the environment set up (only if inside docker)
if [ "$INSIDE_DOCKER" = "true" ]; then
    export PYTHONPATH="${PYTHONPATH:-}:/workspace"
    # Ensure system dependencies are present for specific phases
    case "$PHASE" in
        5|9|11_3|all)
            if ! dpkg -l | grep libsystemc-dev >/dev/null; then
                echo "==> Installing SystemC dependencies..."
                apt-get update -qq && apt-get install -y -qq --no-install-recommends libsystemc-dev >/dev/null
            fi
            ;;
    esac
    
    # Ensure Python dependencies are synced in the container
    mkdir -p target
    if [ ! -f target/.ci_marker_uv_synced ]; then
        echo "==> Syncing Python dependencies inside container..."
        uv pip install --link-mode=copy --system --break-system-packages -r pyproject.toml >/dev/null
        touch target/.ci_marker_uv_synced
    fi

    if [ ! -f target/.ci_marker_artifacts_built ]; then
        echo "==> Building test artifacts..."
        make build-test-artifacts >/dev/null
        touch target/.ci_marker_artifacts_built
    fi
fi

run_phase() {
    local p=$1
    echo "════════════════════════════════════════════════════"
    echo "  Running CI Phase: $p"
    echo "════════════════════════════════════════════════════"

    # Ensure a clean slate before each phase
    bash scripts/cleanup-sim.sh --quiet

    case "$p" in
        1)
            make -C test/phase1 && bash test/phase1/smoke_test.sh
            ;;
        2)
            bash test/phase2/smoke_test.sh
            ;;
        3)
            bash test/phase3/smoke_test.sh
            ;;
        3.5)
            make -C test/phase1 && bash test/phase3.5/smoke_test.sh
            ;;
        4)
            make -C test/phase1 && bash test/phase4/smoke_test.sh
            ;;
        5)
            bash test/phase5/smoke_test.sh
            ;;
        6)
            bash test/phase6/smoke_test.sh
            ;;
        7)
            bash test/phase7/smoke_test.sh
            ;;
        8)
            make -C test/phase1 && make -C test/phase8 && bash test/phase8/smoke_test.sh
            ;;
        9)
            make -C test/phase1 && bash test/phase9/smoke_test.sh
            ;;
        10)
            make -C test/phase1 && bash test/phase10/smoke_test.sh
            ;;
        11)
            make -C test/riscv && bash test/phase11/smoke_test.sh
            ;;
        11_3)
            cmake -S tools/systemc_adapter -B tools/systemc_adapter/build -DCMAKE_BUILD_TYPE=Release >/dev/null
            make -C tools/systemc_adapter/build rp_adapter >/dev/null
            bash test/phase11_3/smoke_test.sh
            ;;
        12)
            make -C test/phase1 && make -C test/phase12 && bash test/phase12/smoke_test.sh
            ;;
        13)
            bash test/phase13/smoke_test.sh
            ;;
        14)
            bash test/phase14/smoke_test.sh
            ;;
        16)
            make -C test/phase1 && make -C test/phase16 && bash test/phase16/smoke_test.sh
            ;;
        actuator)
            bash test/actuator/smoke_test.sh
            ;;
        27)
            PYTHONPATH=$(pwd) pytest tests/test_flexray.py -v --tb=short
            ;;
        qmp)
            make -C test/phase1 && make -C test/phase8 && pytest tools/testing/test_qmp.py -v --tb=short
            ;;
        robot)
            make -C test/phase1 && make -C test/phase8 && robot --outputdir test-results/robot --xunit test-results/robot.xml tests/test_qmp_keywords.robot tests/test_interactive_echo.robot
            ;;
        *)
            echo "ERROR: Unknown phase '$p'"
            exit 1
            ;;
    esac
}

if [ "$PHASE" = "all" ]; then
    # The authoritative list of phases that MUST pass
    # (Matches the matrix in .github/workflows/ci.yml)
    for p in 1 2 3 3.5 4 5 6 7 8 9 10 11 11_3 12 13 14 16 actuator 27 qmp robot; do
        run_phase "$p"
    done
else
    run_phase "$PHASE"
fi
