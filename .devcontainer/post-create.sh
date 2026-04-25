#!/usr/bin/env bash
set -euo pipefail

echo "==> Configuring Git..."
git config --global credential.https://github.com.helper ''
git config --global --unset-all credential.helper || true
git config --global --add credential.helper ''
git config --global --add credential.helper '!gh auth git-credential'
git config --global --add safe.directory /workspace
git config --global core.pager less

# Self-healing: Switch SSH remote to HTTPS if needed.
# SSH agent forwarding frequently breaks on macOS/Windows during sleep or Docker restarts.
# HTTPS + Git Credential Helper is the officially recommended, bulletproof way for DevContainers.
CURRENT_REMOTE=$(git remote get-url origin 2>/dev/null || echo "")
if [[ "$CURRENT_REMOTE" == git@github.com:RefractSystems/virtmcu.git ]]; then
    echo "    Detecting SSH remote. Switching to HTTPS for reliable DevContainer authentication..."
    git remote set-url origin https://github.com/RefractSystems/virtmcu.git
fi

# Fix stale Docker credsStore/credHelpers injected by VS Code if it exists
if [ -f ~/.docker/config.json ]; then
    echo "    Cleaning up Docker config.json to prevent credential helper errors..."
    # Use jq if available for robust JSON manipulation
    if command -v jq >/dev/null 2>&1; then
        TMP_DOCKER_CONFIG=$(mktemp)
        jq 'del(.credsStore, .credHelpers)' ~/.docker/config.json > "$TMP_DOCKER_CONFIG" && mv "$TMP_DOCKER_CONFIG" ~/.docker/config.json || rm -f "$TMP_DOCKER_CONFIG"
    else
        # Fallback to sed if jq is missing (less robust)
        sed -i '/"credsStore":/d' ~/.docker/config.json
        sed -i '/"credHelpers":/d' ~/.docker/config.json
        # Clean up empty lines or dangling commas
        sed -i 's/,,/,/g' ~/.docker/config.json
        sed -i 's/{,/{/g' ~/.docker/config.json
        sed -i 's/,}/}/g' ~/.docker/config.json
        # Handle trailing commas across newlines (basic attempt)
        sed -i ':a;N;$!ba;s/,\s*}/\n}/g' ~/.docker/config.json
    fi
fi

# Set Git identity if missing globally
if [ -z "$(git config --global user.email)" ]; then
    echo "    Detecting Git identity from GitHub..."
    GH_USER_JSON=$(gh api user 2>/dev/null || echo "{}")
    if [ "$GH_USER_JSON" != "{}" ]; then
        GH_NAME=$(echo "$GH_USER_JSON" | jq -r '.name // .login')
        GH_EMAIL=$(echo "$GH_USER_JSON" | jq -r '.email // empty')
        
        if [ -z "$GH_EMAIL" ]; then
            GH_LOGIN=$(echo "$GH_USER_JSON" | jq -r '.login')
            GH_ID=$(echo "$GH_USER_JSON" | jq -r '.id')
            GH_EMAIL="${GH_ID}+${GH_LOGIN}@users.noreply.github.com"
        fi
        
        git config --global user.name "$GH_NAME"
        git config --global user.email "$GH_EMAIL"
        echo "    Set Git identity to: $GH_NAME <$GH_EMAIL>"
    else
        echo "    Warning: Could not detect GitHub identity. Please run 'git config --global user.email \"you@example.com\"'."
    fi
fi

echo "==> Synchronizing Python Environment..."
uv sync

echo "==> Configuring shell environment..."
for RC_FILE in ~/.zshrc ~/.bashrc; do
    if [ -f "$RC_FILE" ]; then
        grep -q "VIRTUAL_ENV/bin/activate" "$RC_FILE" || echo '[ -f /workspace/.venv/bin/activate ] && source /workspace/.venv/bin/activate' >> "$RC_FILE"
        grep -q "source /workspace/.env" "$RC_FILE" || echo 'set -a; [ -f /workspace/.env ] && source /workspace/.env; set +a' >> "$RC_FILE"
        grep -q "alias gemini=" "$RC_FILE" || echo "alias gemini='gemini --yolo'" >> "$RC_FILE"
        grep -q "export PATH.*.local/bin" "$RC_FILE" || echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$RC_FILE"
        grep -q "export PAGER=less" "$RC_FILE" || echo 'export PAGER=less' >> "$RC_FILE"
    fi
done

echo "==> Installing Git Hooks..."
make install-hooks

echo "==> Initializing Workspace Dependencies..."
# Ensure /workspace is a safe directory (idempotent)
git config --global --replace-all safe.directory /workspace
# This is fast if QEMU is pre-installed in the container.
# In transition periods, an old container image may carry a stale QEMU version.
# We allow setup to proceed with a warning so downstream init steps (git hooks,
# Python env, etc.) are never blocked by a version mismatch.
if ! make setup-initial; then
    REQUIRED_VER=$(grep '^QEMU_VERSION=' BUILD_DEPS 2>/dev/null | cut -d= -f2 || echo "unknown")
    echo ""
    echo "⚠️  WARNING: QEMU setup did not fully complete."
    echo "    The container image may pre-date the QEMU ${REQUIRED_VER} upgrade."
    echo "    Simulation features may be limited with the installed version."
    echo "    To rebuild QEMU from source: ./scripts/setup-qemu.sh --force"
    echo ""
fi

echo "✓ DevContainer initialization complete."
