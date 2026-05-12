#!/usr/bin/env bash
set -euo pipefail

echo "==> Seeding Claude configuration..."
HOST_CLAUDE_JSON="/home/vscode/.claude.json.host" # virtmcu-allow: absolute_path reasoning="Legacy script"
DEST_CLAUDE_JSON="/home/vscode/.claude.json" # virtmcu-allow: absolute_path reasoning="Legacy script"
if [ -f "$HOST_CLAUDE_JSON" ] && python3 -c "import json; json.load(open('$HOST_CLAUDE_JSON'))" 2>/dev/null; then
    cp "$HOST_CLAUDE_JSON" "$DEST_CLAUDE_JSON"
    echo "    Copied valid .claude.json from host."
else
    echo "    Host .claude.json missing or invalid JSON — starting with empty config."
    echo '{}' > "$DEST_CLAUDE_JSON"
fi

echo "✓ AI configuration seeded."

# Self-healing: Fix stale Docker credsStore/credHelpers re-injected by VS Code on startup
if [ -f ~/.docker/config.json ]; then
    if command -v jq >/dev/null 2>&1; then
        if jq -e '.credsStore or .credHelpers' ~/.docker/config.json >/dev/null 2>&1; then
            echo "==> Cleaning up re-injected Docker credential helpers..."
            TMP_DOCKER_CONFIG=$(mktemp)
            jq 'del(.credsStore, .credHelpers)' ~/.docker/config.json > "$TMP_DOCKER_CONFIG" && mv "$TMP_DOCKER_CONFIG" ~/.docker/config.json || rm -f "$TMP_DOCKER_CONFIG"
        fi
    fi
fi

echo "✓ Container start complete."

# Self-healing: Ensure Docker daemon is running (important for some DinD environments)
if command -v docker >/dev/null 2>&1; then
    if ! docker info >/dev/null 2>&1; then
        echo "==> Docker daemon not running. Attempting to start..."
        # Try both service and direct init.d as fallback
        sudo service docker start || sudo /etc/init.d/docker start || true
        
        # Wait for daemon to be ready (max 10 seconds)
        for i in {1..10}; do
            if docker info >/dev/null 2>&1; then
                echo "✓ Docker daemon started successfully."
                break
            fi
            sleep 1
        done
        
        if ! docker info >/dev/null 2>&1; then
            echo "⚠️ Warning: Docker daemon failed to start. 'make docker-*' targets may fail."
        fi
    else
        echo "✓ Docker daemon is already running."
    fi
fi
