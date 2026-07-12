#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="$HOME/tools/bin"

# Parse arguments
BUILD=false
PORT=8080

while [[ $# -gt 0 ]]; do
    case $1 in
        --build) BUILD=true; shift ;;
        --port) PORT="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Build if requested or if binaries don't exist
if [ "$BUILD" = true ] || [ ! -f "$INSTALL_DIR/graphite" ]; then
    echo "Building Graphite..."
    "$SCRIPT_DIR/build-and-install.sh"
    echo ""
fi

# Check that graphite exists
if [ ! -f "$INSTALL_DIR/graphite" ]; then
    echo "Error: graphite not found at $INSTALL_DIR/graphite"
    echo "Run: ./build-and-install.sh"
    exit 1
fi

echo "Starting Graphite with MCP server on port $PORT..."
echo ""
echo "The Graphite window will open. The agent can connect via:"
echo "  graphite-mcp-client"
echo ""
echo "Press Ctrl+C to stop."
echo ""

# Start Graphite with MCP server
"$INSTALL_DIR/graphite" --mcp-server --mcp-port "$PORT"
