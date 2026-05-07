#!/usr/bin/env bash
set -euo pipefail

# Self-healing: Switch SSH remote to HTTPS if needed.
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

echo "==> Fixing Docker volume permissions..."
# Docker creates volumes as root by default. Fix permissions for Cargo caches.
sudo chown -R vscode:vscode /usr/local/cargo/registry /workspace/target 2>/dev/null || true

echo "==> Configuring shell environment..."
for RC_FILE in ~/.zshrc ~/.bashrc; do
    if [ -f "$RC_FILE" ]; then
        grep -q "source /workspace/.env" "$RC_FILE" || echo 'set -a; [ -f /workspace/.env ] && source /workspace/.env; set +a' >> "$RC_FILE"
        
        # Devcontainer Aliases
        grep -q "alias gemini=" "$RC_FILE" || echo "alias gemini='gemini --yolo'" >> "$RC_FILE"
        grep -q "alias git=" "$RC_FILE" || echo "alias git='git --no-pager'" >> "$RC_FILE"
        
        grep -q "export PATH.*.local/bin" "$RC_FILE" || echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$RC_FILE"
        grep -q "export PAGER=less" "$RC_FILE" || echo 'export PAGER=less' >> "$RC_FILE"
    fi
done

echo "==> Installing Git Hooks..."
make install-git-hooks

echo "==> Initializing Workspace Dependencies..."
# Ensure /workspace is a safe directory (idempotent)
# We use --system to avoid modifying the (potentially read-only) global .gitconfig
# and to keep container-specific paths out of the host's configuration.
sudo git config --system --replace-all safe.directory /workspace
# Since we use devenv, make install-deps is explicitly required for the first run.
# We do not block container startup, but we warn the developer if it fails.
if ! make install-deps-initial; then
    echo ""
    echo "⚠️  WARNING: Initial setup (make install-deps-initial) failed or was interrupted."
    echo "    You MUST run 'make install-deps' manually before running tests or simulations."
    echo ""
fi

echo "✓ DevContainer initialization complete."
