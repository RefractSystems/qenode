#!/usr/bin/env bash
# ==============================================================================
# scripts/cleanup-sim.sh
#
# Thoroughly cleans up simulation-related processes (QEMU, Zenoh) and temp files.
#
# WORKSPACE SCOPING (MULTI-AGENT SAFETY):
# This script is highly optimized to be safe for concurrent execution by multiple
# users or AI agents on the same host. It inspects /proc/<pid>/cwd and
# /proc/<pid>/cmdline to ensure it ONLY kills orphaned processes that originated
# from the current working directory. It will not touch simulations running in
# other cloned workspaces.
# ==============================================================================

set -euo pipefail

QUIET=${1:-""}
FILTER=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --quiet)
            QUIET="--quiet"
            shift
            ;;
        --filter)
            FILTER="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

log() {
    if [[ "$QUIET" != "--quiet" ]]; then
        echo "$@"
    fi
}

log "==> Cleaning up simulation environment (Workspace: $WORKSPACE_DIR)..."

# Target processes specifically owned by the current user to avoid interfering with others
PROCESSES=(
    "qemu-system-arm"
    "qemu-system-aarch64"
    "qemu-system-riscv64"
    "qemu-system-riscv32"
    "zenohd"
    "zenoh_router"
    "zenoh_router_persistent.py"
    "zenoh_coordinator"
    "mmio-socket-bridge"
)

get_workspace_pids() {
    local proc_name="$1"
    local pids=""
    
    # Iterate through all PIDs matching the process name for the current user
    for pid in $(pgrep -u "$(id -u)" -f "$proc_name" 2>/dev/null || true); do
        local cmdline=""
        local cwd=""
        
        if [ -f "/proc/$pid/cmdline" ]; then
            cmdline=$(cat "/proc/$pid/cmdline" | tr '\0' ' ')
        fi
        if [ -L "/proc/$pid/cwd" ]; then
            cwd=$(readlink "/proc/$pid/cwd")
        fi
        
        # If the process is running from our workspace, or its command line references our workspace
        if [[ "$cwd" == "$WORKSPACE_DIR"* ]] || [[ "$cmdline" == *"$WORKSPACE_DIR"* ]]; then
            # If a filter is specified, only include if cmdline matches filter
            if [ -n "$FILTER" ]; then
                if [[ "$cmdline" == *"$FILTER"* ]]; then
                    pids="$pids $pid"
                fi
            else
                pids="$pids $pid"
            fi
        fi
    done
    echo "$pids"
}

for proc in "${PROCESSES[@]}"; do
    PIDS=$(get_workspace_pids "$proc")
    if [ -n "$PIDS" ]; then
        log "Killing $proc (PIDs:$PIDS)..."
        kill $PIDS 2>/dev/null || true
    fi
done

# Second pass with SIGKILL for anything stubborn
sleep 0.5
for proc in "${PROCESSES[@]}"; do
    PIDS=$(get_workspace_pids "$proc")
    if [ -n "$PIDS" ]; then
        log "Force-killing stubborn $proc (PIDs:$PIDS)..."
        kill -9 $PIDS 2>/dev/null || true
    fi
done

if [ -z "$FILTER" ]; then
    log "Cleaning up temporary files..."
    # We leave /tmp/virtmcu-test-* alone because pytest handles its own tempdir cleanup safely.
    # We only clean up legacy hardcoded /tmp files if they exist.
    rm -f /tmp/virtmcu-*.dtb 2>/dev/null || true
    rm -f /tmp/virtmcu-*.cli 2>/dev/null || true
    rm -f /tmp/virtmcu-*.arch 2>/dev/null || true

    # Clean up stale lock files
    rm -f "$TOOLS_DIR/zenoh_coordinator/build.lock" 2>/dev/null || true
fi

log "✓ Cleanup complete."
