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

# Build if requested or if binaries don't exist
if [ "$BUILD" = true ] || [ ! -f "$INSTALL_DIR/graphite-mcp-client" ]; then
    echo "Building graphite-mcp-client..."
    (cd "$SCRIPT_DIR" && rustup run 1.95.0 cargo build --release -p graphite-mcp-client)
    mkdir -p "$INSTALL_DIR"
    cp "$SCRIPT_DIR/target/release/graphite-mcp-client" "$INSTALL_DIR/graphite-mcp-client"
    codesign --force --sign - "$INSTALL_DIR/graphite-mcp-client" 2>/dev/null || true
    echo ""
fi

# Check that graphite-mcp-client exists
if [ ! -f "$INSTALL_DIR/graphite-mcp-client" ]; then
    echo "Error: graphite-mcp-client not found at $INSTALL_DIR/graphite-mcp-client"
    echo "Run: ./run.sh --build"
    exit 1
fi

echo "Starting Graphite MCP relay server on port $MCP_PORT..."
echo ""
echo "The relay waits for the browser to connect, then bridges agent tool calls."
echo ""
echo "Next steps:"
echo "  1. Start the Graphite web app in another terminal:"
echo "       cd \"$SCRIPT_DIR\" && cargo run"
echo "  2. Open http://localhost:$PORT in your browser"
echo "  3. The MCP bridge connects automatically (check browser console)"
echo "  4. Configure your AI agent (opencode/Claude) to use this MCP server"
echo ""
echo "Press Ctrl+C to stop."
echo ""

# Start the MCP relay server
export GRAPHITE_MCP_PORT="$MCP_PORT"
exec "$INSTALL_DIR/graphite-mcp-client"
