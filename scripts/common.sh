#!/usr/bin/env bash
set -euo pipefail
# scripts/common.sh — Centralized helpers for virtmcu scripts.

# SOTA Workspace Root Discovery:
# 1. Try git rev-parse (fastest/most reliable in dev)
# 2. Climb up looking for 'VERSION' marker (works in docker/release bundles)
find_workspace_root() {
    local cur
    cur="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    
    # Try git first
    if command -v git >/dev/null 2>&1; then
        local git_root
        git_root=$(git -C "$cur" rev-parse --show-toplevel 2>/dev/null)
        if [[ -n "$git_root" ]]; then
            echo "$git_root"
            return 0
        fi
    fi

    # Fallback: Climb up
    while [[ "$cur" != "/" ]]; do
        if [[ -f "$cur/VERSION" || -d "$cur/.git" ]]; then
            echo "$cur"
            return 0
        fi
        cur=$(dirname "$cur")
    done
    
    # If we reach here, we are lost. Default to script's grandparent if nothing else works.
    echo "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
}

# Export standard directory variables
WORKSPACE_DIR="$(find_workspace_root)"
SCRIPTS_DIR="$WORKSPACE_DIR/scripts"
TOOLS_DIR="$WORKSPACE_DIR/tools"
RUN_SH="$SCRIPTS_DIR/run.sh"

export WORKSPACE_DIR SCRIPTS_DIR TOOLS_DIR RUN_SH
