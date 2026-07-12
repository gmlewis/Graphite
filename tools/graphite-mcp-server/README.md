# Graphite MCP Server (legacy / catalog-only)

> **⚠️ This document describes the legacy in-process MCP server crate.**
> It is **not** the MCP server that AI agents connect to in normal use.
>
> The authoritative, currently-used MCP server is the **`graphite-mcp-client`**
> relay + the browser-side bridge in `frontend/src/mcp-bridge.ts`. See
> [`../graphite-mcp-client/README.md`](../graphite-mcp-client/README.md) for
> the real setup, tool list, and agent guidance.
>
> This crate (`tools/graphite-mcp-server/`) remains useful only for
> **standalone node-catalog queries** (`graphite-mcp --standalone`). The
> `--mcp` headed mode and the editor-bridge tool handlers described below are
> **stubs** — most return `"(handler not yet implemented)"` (see
> `src/editor_bridge.rs`). Do not rely on them for editor automation.

---

An MCP (Model Context Protocol) server for the Graphite vector graphics editor, giving AI agents full control over the editor.

## Quick Start

### Headed Mode (Recommended)

Launch Graphite with MCP support:

```bash
graphite --mcp
```

This starts the editor with the MCP server on stdin/stdout. Connect your MCP client (e.g., Claude Desktop) to this process.

### Standalone Mode (Catalog Only)

For node catalog queries without the editor:

```bash
graphite-mcp --standalone
```

## Installation

```bash
./build-and-install.sh
```

This builds and installs both binaries to `~/tools/bin/`.

## Claude Desktop Configuration

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "graphite": {
      "command": "/Users/glenn/tools/bin/graphite",
      "args": ["--mcp"],
      "env": {}
    }
  }
}
```

Restart Claude Desktop after adding this configuration.

## Available Tools (24)

### Document Tools

| Tool | Description |
|------|-------------|
| `list_documents` | List all open documents |
| `create_document` | Create a new blank document |
| `get_node_catalog` | Search all 272 nodes with categories |
| `get_node_details` | Get full details for a specific node |

### Layer Tools

| Tool | Description |
|------|-------------|
| `get_layer_tree` | Get full layer hierarchy with IDs, names, visibility |
| `create_rectangle` | Create a rectangle shape |
| `create_ellipse` | Create an ellipse shape |
| `create_line` | Create a line |
| `create_text` | Create a text layer |
| `select_layer` | Select a layer by ID |
| `delete_selected` | Delete selected layers |
| `get_selection` | List selected layer IDs |
| `get_layer_properties` | Get layer properties (name, kind, visibility, lock) |
| `get_node_graph` | Get node graph for a layer |

### Style Operations

| Tool | Description |
|------|-------------|
| `set_fill_color` | Set fill opacity of selected layers |
| `set_stroke` | Set stroke color and width on first selected layer |
| `set_opacity` | Set opacity (0-100) of selected layers |
| `set_blend_mode` | Set blend mode (27 modes supported) |

### Transform Operations

| Tool | Description |
|------|-------------|
| `move_layer` | Move selected layers by dx/dy |

### Tool Operations

| Tool | Description |
|------|-------------|
| `activate_tool` | Activate a tool (Select, Pen, Rectangle, etc.) |

### Viewport Operations

| Tool | Description |
|------|-------------|
| `zoom_to_fit` | Zoom viewport to fit all content |
| `set_viewport` | Set viewport zoom level |

### History

| Tool | Description |
|------|-------------|
| `undo` | Undo the last action |
| `redo` | Redo the last undone action |

## Example Usage

### Create a Document with Shapes

```json
{"method": "tools/call", "params": {"name": "create_document", "arguments": {"name": "My Design"}}}
{"method": "tools/call", "params": {"name": "create_rectangle", "arguments": {"x": 100, "y": 100, "width": 200, "height": 150, "fill_color": "#FF5733"}}}
{"method": "tools/call", "params": {"name": "create_ellipse", "arguments": {"x": 400, "y": 200, "radius_x": 80, "radius_y": 80, "fill_color": "#33FF57"}}}
```

### Modify Styles

```json
{"method": "tools/call", "params": {"name": "set_opacity", "arguments": {"opacity": 75}}}
{"method": "tools/call", "params": {"name": "set_blend_mode", "arguments": {"blend_mode": "Multiply"}}}
```

### Navigate the Canvas

```json
{"method": "tools/call", "params": {"name": "zoom_to_fit", "arguments": {}}}
{"method": "tools/call", "params": {"name": "set_viewport", "arguments": {"zoom": 2.0}}}
```

## Architecture

The MCP server runs in-process within the Graphite desktop app:

1. `graphite --mcp` launches the editor and starts the MCP server thread
2. MCP thread reads JSON-RPC from stdin
3. Tool calls are sent to the main thread via `AppEvent::McpToolCall`
4. Main thread dispatches to the editor's message system
5. Responses are sent back via a oneshot channel

This means the MCP server has full access to the editor's state and can perform any operation the GUI can.

## Building from Source

```bash
cd /path/to/Graphite
./build-and-install.sh
```

Requires Rust 1.95+ (via rustup).

## Node Catalog

The MCP server includes an embedded node catalog with 272 nodes across 31 categories. This is generated from the source code by `tools/node_catalog_generator.py`.

To regenerate:
```bash
python3 tools/node_catalog_generator.py
```
