#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="$HOME/tools/bin"

# Parse arguments
BUILD=false
PORT=8080
MCP_PORT=8081

while [[ $# -gt 0 ]]; do
    case $1 in
        --build) BUILD=true; shift ;;
        --port) PORT="$2"; shift 2 ;;
        --mcp-port) MCP_PORT="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Build if requested or if the MCP relay doesn't exist
if [ "$BUILD" = true ] || [ ! -f "$INSTALL_DIR/graphite-mcp-client" ]; then
    echo "Building graphite-mcp-client..."
    (cd "$SCRIPT_DIR" && rustup run 1.95.0 cargo build --release -p graphite-mcp-client)
    mkdir -p "$INSTALL_DIR"
    cp "$SCRIPT_DIR/target/release/graphite-mcp-client" "$INSTALL_DIR/graphite-mcp-client"
    codesign --force --sign - "$INSTALL_DIR/graphite-mcp-client" 2>/dev/null || true
    echo ""
fi

if [ ! -f "$INSTALL_DIR/graphite-mcp-client" ]; then
    echo "Error: graphite-mcp-client not found at $INSTALL_DIR/graphite-mcp-client"
    echo "Run: ./run.sh --build"
    exit 1
fi

# Check npm dependencies
if [ ! -d "$SCRIPT_DIR/frontend/node_modules" ]; then
    echo "Installing npm dependencies..."
    (cd "$SCRIPT_DIR/frontend" && npm ci --include=dev --prefer-offline --no-audit --no-fund)
    echo ""
fi

# Check branding assets
if [ ! -d "$SCRIPT_DIR/branding/assets" ]; then
    echo "Downloading branding assets..."
    URL=$(head -1 "$SCRIPT_DIR/.branding")
    curl -L "$URL" -o /tmp/branding.tar.gz
    mkdir -p "$SCRIPT_DIR/branding"
    tar xzf /tmp/branding.tar.gz --strip-components=1 -C "$SCRIPT_DIR/branding/"
    cp "$SCRIPT_DIR/.branding" "$SCRIPT_DIR/branding/.branding"
    echo ""
fi

# Check if WASM wrapper is built
if [ ! -f "$SCRIPT_DIR/frontend/wrapper/pkg/graphite_wasm_wrapper.js" ]; then
    echo "Building WASM wrapper..."
    (cd "$SCRIPT_DIR" && rustup run 1.95.0 cargo build --lib --package graphite-wasm-wrapper --target wasm32-unknown-unknown)
    rustup run 1.95.0 wasm-bindgen --target web --out-name graphite_wasm_wrapper --out-dir "$SCRIPT_DIR/frontend/wrapper/pkg" \
        "$SCRIPT_DIR/target/wasm32-unknown-unknown/debug/graphite_wasm_wrapper.wasm" --debug
    echo ""
fi

echo "Starting Graphite web app (Vite dev server on port $PORT)..."
echo ""
echo "  1. Open http://localhost:$PORT in your browser"
echo "  2. Start your AI agent (opencode) — it spawns graphite-mcp-client"
echo "     which starts the MCP WebSocket relay on port $MCP_PORT"
echo "  3. The browser's MCP bridge auto-connects to the relay"
echo "     (check browser console for '[MCP Bridge] Connected')"
echo ""
echo "Press Ctrl+C to stop the web app."
echo ""

# Start Vite dev server (the agent will spawn graphite-mcp-client separately)
cd "$SCRIPT_DIR/frontend"
CARGO_TARGET_DIR="$SCRIPT_DIR/target" exec npx vite --port "$PORT" --host 0.0.0.0
