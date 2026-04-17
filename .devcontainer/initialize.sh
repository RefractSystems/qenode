#!/usr/bin/env bash
# Runs locally on the host machine before the devcontainer is built/started.

set -e

# 1. Ensure host directories and files exist for bind mounts
mkdir -p ~/.claude ~/.gemini ~/.config/gh
touch ~/.claude.json

# 2. Fetch and print the cache image digest to the devcontainer logs
echo -e "\n\n====== PULLING DEVENV CACHE ======"
if docker pull -q ghcr.io/refractsystems/virtmcu/devenv:latest > /dev/null 2>&1; then
    docker inspect --format="{{index .RepoDigests 0}}" ghcr.io/refractsystems/virtmcu/devenv:latest
else
    echo "Failed to fetch cache image digest"
fi
echo -e "===================================\n\n"
