#!/usr/bin/env bash
set -euo pipefail

TARGET_UID=${HOST_UID:-1000}
TARGET_GID=${HOST_GID:-1000}

# 1. Ensure the 'vscode' user matches the host's UID/GID
if [ "$(id -u vscode)" != "$TARGET_UID" ]; then
    echo "Updating vscode UID to $TARGET_UID"
    usermod -u "$TARGET_UID" vscode
fi
if [ "$(id -g vscode)" != "$TARGET_GID" ]; then
    echo "Updating vscode GID to $TARGET_GID"
    # If the TARGET_GID is already taken by another group, change that group's GID first
    EXISTING_GROUP=$(getent group "$TARGET_GID" | cut -d: -f1 || true)
    if [ -z "$EXISTING_GROUP" ]; then
        # Fallback to searching /etc/group if getent failed
        EXISTING_GROUP=$(grep ":x:$TARGET_GID:" /etc/group | cut -d: -f1 || true)
    fi

    if [ -n "$EXISTING_GROUP" ] && [ "$EXISTING_GROUP" != "vscode" ]; then
        echo "GID $TARGET_GID is taken by group $EXISTING_GROUP. Moving $EXISTING_GROUP to a new GID."
        # Find a free GID starting from 9999
        NEW_GID=9999
        while getent group "$NEW_GID" >/dev/null || grep ":x:$NEW_GID:" /etc/group >/dev/null; do
            NEW_GID=$((NEW_GID + 1))
        done
        groupmod -g "$NEW_GID" "$EXISTING_GROUP" || true
    fi
    groupmod -g "$TARGET_GID" vscode
fi

# 2. Fix Volume Permissions
# The Cargo registry volume might have been created with a different UID in a previous run
if [ -d "/usr/local/cargo/registry" ] && [ "$(stat -c %u /usr/local/cargo/registry)" != "$TARGET_UID" ]; then
    echo "Fixing Cargo registry permissions..."
    chown -R vscode:vscode /usr/local/cargo/registry
fi

if [ -d "/workspace/target" ] && [ "$(stat -c %u /workspace/target)" != "$TARGET_UID" ]; then
    echo "Fixing workspace target permissions..."
    chown -R vscode:vscode /workspace/target
fi

# 3. Drop privileges and execute the command passed to docker run
exec gosu vscode "$@"