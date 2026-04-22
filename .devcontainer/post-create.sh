#!/usr/bin/env bash
set -e

echo "==> Configuring Git..."
git config --global credential.https://github.com.helper ''
git config --global --unset-all credential.helper || true
git config --global --add credential.helper ''
git config --global --add credential.helper '!gh auth git-credential'
git config --global --add safe.directory /workspace

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
    # Remove credsStore and credHelpers which often point to host-only binaries
    sed -i '/"credsStore":/d' ~/.docker/config.json
    sed -i '/"credHelpers":/d' ~/.docker/config.json
    # Clean up empty lines or dangling commas that might break JSON (basic cleanup)
    sed -i 's/,,/,/g' ~/.docker/config.json
    sed -i 's/{,/{/g' ~/.docker/config.json
    sed -i 's/,}/}/g' ~/.docker/config.json
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
echo '[ -f /workspace/.venv/bin/activate ] && source /workspace/.venv/bin/activate' >> ~/.zshrc
echo 'set -a; [ -f /workspace/.env ] && source /workspace/.env; set +a' >> ~/.zshrc

echo "==> Installing AI Developer Tools (Claude Code & Gemini CLI)..."
sudo npm install -g @google/gemini-cli@latest
curl -fsSL https://claude.ai/install.sh | bash
echo "alias gemini='gemini --yolo'" >> ~/.zshrc
echo "export PATH=\"\$HOME/.local/bin:\$PATH\"" >> ~/.zshrc

echo "==> Installing Git Hooks..."
make install-hooks

echo "==> Initializing Workspace Dependencies..."
# This is fast if QEMU is pre-installed in the container
make setup-initial

echo "✓ DevContainer initialization complete."
