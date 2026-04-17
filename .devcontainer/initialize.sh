#!/usr/bin/env bash
# Runs locally on the host machine before the devcontainer is built/started.

set -e

# 1. Ensure host directories and files exist for bind mounts
mkdir -p ~/.claude ~/.gemini ~/.config/gh
touch ~/.claude.json

# 2. Fetch and print the cache image digest to the devcontainer logs
echo -e "\n\n====== PULLING DEVENV CACHE ======"
IMAGE="ghcr.io/refractsystems/virtmcu/devenv:latest"

if command -v docker >/dev/null 2>&1; then
    echo "Fetching $IMAGE (this may take a minute)..."
    
    # Run docker pull and add timestamps to each line. 
    # We use a subshell to provide a "heartbeat" if there is no output for a while.
    (
        docker pull "$IMAGE" 2>&1 | while read -r line; do
            echo "[$(date +%H:%M:%S)] $line"
        done
    ) &
    PULL_PID=$!

    # Heartbeat: print a "." every 10 seconds of silence to show the script is alive
    while kill -0 $PULL_PID 2>/dev/null; do
        sleep 10
        if kill -0 $PULL_PID 2>/dev/null; then
            echo -e "[$(date +%H:%M:%S)] ... still pulling $IMAGE"
        fi
    done

    if wait $PULL_PID; then
        echo -n "Digest: "
        docker inspect --format="{{index .RepoDigests 0}}" "$IMAGE"
    else
        echo "Failed to fetch cache image: $IMAGE"
    fi
else
    echo "Docker not found, skipping cache pull."
fi
echo -e "===================================\n\n"
