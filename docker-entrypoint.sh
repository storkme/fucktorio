#!/bin/bash
set -e

# ---------------------------------------------------------------------------
# GitHub auth — pass GH_TOKEN at `docker run` time:
#   docker run -e GH_TOKEN=ghp_... ...
# gh picks up GH_TOKEN automatically; just verify it works.
# ---------------------------------------------------------------------------
if [ -n "$GH_TOKEN" ]; then
    echo "gh: authenticated as $(gh api user --jq .login)"
    git config --global --unset-all credential.https://github.com.helper 2>/dev/null || true
    git config --global --unset-all credential.https://gist.github.com.helper 2>/dev/null || true
    gh auth setup-git
else
    echo "Warning: GH_TOKEN not set — gh will be unauthenticated"
fi

# ---------------------------------------------------------------------------
# Clone or pull fucktorio
# ---------------------------------------------------------------------------
REPO_DIR=/workspace/fucktorio

if [ ! -d "$REPO_DIR/.git" ]; then
    echo "Cloning storkme/fucktorio..."
    gh repo clone storkme/fucktorio "$REPO_DIR"
else
    echo "Repo already present, pulling latest..."
    git -C "$REPO_DIR" pull --ff-only
fi

cd "$REPO_DIR"

# ---------------------------------------------------------------------------
# Install steps (from CLAUDE.md)
# ---------------------------------------------------------------------------

echo "--- uv sync ---"
uv sync

echo "--- maturin develop (PyO3 A* extension) ---"
uvx maturin develop --manifest-path crates/pyo3-bindings/Cargo.toml

echo "--- wasm-pack (WASM bundle for web app) ---"
wasm-pack build crates/wasm-bindings --target web \
    --out-dir "$REPO_DIR/web/src/wasm-pkg"

echo "--- npm install (web app) ---"
npm install --prefix web

echo ""
echo "Ready. Working directory: $REPO_DIR"
cd "$REPO_DIR"

exec "$@"
