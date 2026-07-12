#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="$HOME/tools/bin"

echo "Building Graphite MCP Client (WebSocket relay)..."
echo ""

# Build the lightweight MCP relay (stdio <-> WebSocket bridge)
echo "→ Building graphite-mcp-client (MCP relay server)..."
rustup run 1.95.0 cargo build --release -p graphite-mcp-client
echo "  ✓ graphite-mcp-client"
echo ""

# Install to ~/tools/bin
mkdir -p "$INSTALL_DIR"
cp target/release/graphite-mcp-client "$INSTALL_DIR/graphite-mcp-client"
echo "Installed: $INSTALL_DIR/graphite-mcp-client"

# Ad-hoc codesign so macOS doesn't kill the process
echo ""
echo "Signing binary..."
codesign --force --sign - "$INSTALL_DIR/graphite-mcp-client"
echo "  ✓ codesigned"

echo ""
echo "Done! Installed to $INSTALL_DIR"
echo ""
echo "Usage (Web app + MCP pattern):"
echo "  Terminal 1: Start the web app"
echo "    cd frontend && cargo run   # builds WASM + starts Vite dev server on :8080"
echo "  Terminal 2: Start the MCP relay"
echo "    ./run.sh                   # starts MCP relay on :8081"
echo "  Open http://localhost:8080 in your browser"
echo "  Configure your AI agent to use graphite-mcp-client as an MCP server"
echo ""
echo "Make sure $INSTALL_DIR is in your PATH:"
echo "  export PATH=\"\$HOME/tools/bin:\$PATH\"