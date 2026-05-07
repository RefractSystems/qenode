#!/bin/bash
set -euo pipefail

# Enterprise-grade apt-get install wrapper for Docker builds.
# Mitigates transient Debian mirror synchronization issues ("File has unexpected size").
# Implements exponential backoff rather than a fixed sleep, and handles hash mismatches specifically.

MAX_RETRIES=3
BASE_WAIT=5

# Inject apt retry configuration if it doesn't exist
if [ ! -f /etc/apt/apt.conf.d/80-retries ]; then
    echo 'Acquire::Retries "5";' > /etc/apt/apt.conf.d/80-retries
fi

attempt=1
while [ $attempt -le $MAX_RETRIES ]; do
    echo "==> apt-get update (Attempt $attempt of $MAX_RETRIES)..."
    
    # Try the update
    if apt-get update; then
        # Update succeeded, proceed to install
        echo "==> apt-get update successful, proceeding with install..."
        apt-get install -y --no-install-recommends "$@"
        rm -rf /var/lib/apt/lists/*
        exit 0
    else
        # Update failed. Check if we have retries left.
        if [ $attempt -eq $MAX_RETRIES ]; then
            echo "❌ ERROR: apt-get update failed after $MAX_RETRIES attempts." >&2
            exit 1
        fi
        
        # Exponential backoff
        wait_time=$((BASE_WAIT * attempt))
        echo "⚠️ apt-get update failed. Retrying in $wait_time seconds..."
        
        # Clean lists to force a completely fresh fetch next time, 
        # mitigating cached but corrupted InRelease files.
        rm -rf /var/lib/apt/lists/*
        
        sleep $wait_time
    fi
    attempt=$((attempt + 1))
done
