#!/usr/bin/env bash
set -e

echo "==> Seeding Claude configuration..."
HOST_CLAUDE_JSON="/home/vscode/.claude.json.host"
DEST_CLAUDE_JSON="/home/vscode/.claude.json"
if [ -f "$HOST_CLAUDE_JSON" ] && python3 -c "import json; json.load(open('$HOST_CLAUDE_JSON'))" 2>/dev/null; then
    cp "$HOST_CLAUDE_JSON" "$DEST_CLAUDE_JSON"
    echo "    Copied valid .claude.json from host."
else
    echo "    Host .claude.json missing or invalid JSON — starting with empty config."
    echo '{}' > "$DEST_CLAUDE_JSON"
fi

echo "==> Ensuring AI developer tools are installed..."

if ! command -v claude &>/dev/null; then
    echo "    Installing Claude Code..."
    curl -fsSL https://claude.ai/install.sh | bash
else
    echo "    Claude Code already installed: $(claude --version 2>/dev/null || echo 'unknown version')"
fi

if ! command -v gemini &>/dev/null; then
    echo "    Installing Gemini CLI..."
    sudo npm install -g @google/gemini-cli@latest
else
    echo "    Gemini CLI already installed."
fi

echo "✓ Container start complete."
