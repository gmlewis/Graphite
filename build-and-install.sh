#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="$HOME/tools/bin"

echo "Building Graphite MCP Server (release)..."
echo ""

# Build the desktop app with MCP support (the headed MCP server)
echo "→ Building graphite (desktop app + MCP server)..."
rustup run 1.95.0 cargo build --release -p graphite-desktop --features mcp
echo "  ✓ graphite"
echo ""

# Build the standalone MCP server (catalog-only, no editor)
echo "→ Building graphite-mcp (standalone catalog server)..."
rustup run 1.95.0 cargo build --release -p graphite-mcp-server
echo "  ✓ graphite-mcp"
echo ""

# Build the lightweight MCP client proxy
echo "→ Building graphite-mcp-client (WebSocket proxy)..."
rustup run 1.95.0 cargo build --release -p graphite-mcp-client
echo "  ✓ graphite-mcp-client"
echo ""

# Install to ~/tools/bin
mkdir -p "$INSTALL_DIR"

cp target/release/graphite "$INSTALL_DIR/graphite"
cp target/release/graphite-mcp "$INSTALL_DIR/graphite-mcp"
cp target/release/graphite-mcp-client "$INSTALL_DIR/graphite-mcp-client"
echo "Installed: $INSTALL_DIR/graphite"
echo "Installed: $INSTALL_DIR/graphite-mcp"
echo "Installed: $INSTALL_DIR/graphite-mcp-client"

# Ad-hoc codesign so macOS doesn't kill the process
echo ""
echo "Signing binaries..."
codesign --force --sign - "$INSTALL_DIR/graphite"
codesign --force --sign - "$INSTALL_DIR/graphite-mcp"
codesign --force --sign - "$INSTALL_DIR/graphite-mcp-client"
echo "  ✓ codesigned"

echo ""
echo "Done! Installed to $INSTALL_DIR"
echo ""
echo "Usage (Blender MCP pattern):"
echo "  1. Start Graphite:  graphite --mcp-server"
echo "  2. Start MCP client: graphite-mcp-client"
echo "  3. Connect Claude Desktop to graphite-mcp-client"
echo ""
echo "Or for headless mode (no GUI):"
echo "  graphite --mcp"
echo ""
echo "Make sure $INSTALL_DIR is in your PATH:"
echo "  export PATH=\"\$HOME/tools/bin:\$PATH\""
