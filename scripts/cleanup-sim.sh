#!/usr/bin/env bash
# scripts/cleanup-sim.sh — Thoroughly clean up simulation-related processes and temporary files.

set -u

QUIET=${1:-""}

log() {
    if [[ "$QUIET" != "--quiet" ]]; then
        echo "$@"
    fi
}

log "==> Cleaning up simulation environment..."

PROCESSES=(
    "qemu-system-arm"
    "qemu-system-aarch64"
    "qemu-system-riscv"
    "zenoh_router"
    "zenoh_router_persistent.py"
    "zenoh_coordinator"
    "mmio-socket-bridge"
)

for proc in "${PROCESSES[@]}"; do
    if pgrep -f "$proc" > /dev/null; then
        log "Killing $proc..."
        # Try SIGTERM first, then SIGKILL
        pkill -f "$proc" 2>/dev/null || true
        sleep 0.1
        pkill -9 -f "$proc" 2>/dev/null || true
    fi
done

log "Cleaning up temporary files..."
rm -rf /tmp/phase[0-9]* 2>/dev/null || true
rm -f /tmp/virtmcu-*.dtb 2>/dev/null || true
rm -f /tmp/virtmcu-*.cli 2>/dev/null || true
rm -f /tmp/virtmcu-*.arch 2>/dev/null || true
rm -f /tmp/*.sock 2>/dev/null || true

log "✓ Cleanup complete."
