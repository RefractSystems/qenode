#!/usr/bin/env bash
# Runs locally on the host machine before the devcontainer is built/started.

set -euo pipefail

# 1. Ensure host directories and files exist for bind mounts
mkdir -p ~/.claude ~/.gemini ~/.config/gh
touch ~/.claude.json

# 2. Fetch and print the cache image digest to the devcontainer logs
echo -e "\n\n====== PULLING DEVENV CACHE ======"
IMAGE="ghcr.io/refractsystems/virtmcu/devenv:latest"

if command -v docker >/dev/null 2>&1; then
    echo "Fetching $IMAGE (this may take a minute)..."
    
    # Run docker pull and add timestamps. 
    # Use pipefail so docker pull errors aren't masked by the while loop.
    (
        set -o pipefail
        docker pull "$IMAGE" 2>&1 | while read -r line; do
            echo "[$(date +%H:%M:%S)] $line"
        done
    ) &
    PULL_PID=$!

    # Clean up the background process if the user cancels the VS Code build
    trap 'kill $PULL_PID 2>/dev/null' EXIT INT TERM

    # Heartbeat and Timeout (10 minutes = 600 seconds)
    ELAPSED=0
    TIMEOUT=600 
    while kill -0 $PULL_PID 2>/dev/null; do
        sleep 10
        ELAPSED=$((ELAPSED + 10))
        if kill -0 $PULL_PID 2>/dev/null; then
            if [ "$ELAPSED" -ge "$TIMEOUT" ]; then
                echo -e "[$(date +%H:%M:%S)] ERROR: Docker pull timed out after ${TIMEOUT}s. Cancelling."
                kill $PULL_PID 2>/dev/null
                break
            fi
            echo -e "[$(date +%H:%M:%S)] ... still pulling $IMAGE (${ELAPSED}s)"
        fi
    done

    # Remove the trap since we are done waiting
    trap - EXIT INT TERM

    if wait $PULL_PID 2>/dev/null; then
        echo -n "Digest: "
        docker inspect --format="{{index .RepoDigests 0}}" "$IMAGE" || true
    else
        echo "Failed to fetch cache image: $IMAGE. Proceeding without cache."
    fi
else
    echo "Docker not found, skipping cache pull."
fi
echo -e "===================================\n\n"
