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
    echo "Fetching $IMAGE..."
    if docker pull "$IMAGE"; then
        echo -n "Digest: "
        docker inspect --format="{{index .RepoDigests 0}}" "$IMAGE"
    else
        echo "Failed to fetch cache image: $IMAGE"
    fi
else
    echo "Docker not found, skipping cache pull."
fi
echo -e "===================================\n\n"
