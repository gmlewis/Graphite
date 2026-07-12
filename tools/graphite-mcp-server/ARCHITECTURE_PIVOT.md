# Graphite MCP Server — Architecture Pivot Plan

## New Architecture (Blender MCP Pattern)

```
┌──────────────────────────────────────────────────────────────────┐
│  User's Machine                                                  │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  Graphite Desktop App (user starts this first)              │ │
│  │  ├── Full GUI (CEF window)                                  │ │
│  │  ├── WebSocket Server on localhost:8080                     │ │
│  │  └── Editor Backend (message system)                        │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                              ↕ WebSocket                         │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  MCP Client (lightweight, starts with agent)                │ │
│  │  ├── Connects to Graphite WebSocket                         │ │
│  │  ├── Accepts MCP tool calls via stdio                       │ │
│  │  └── Forwards to Graphite, returns results                  │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                              ↕ stdio                             │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  Agent (Claude Desktop / OpenCode / MiMoCode)               │ │
│  └─────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

## Key Differences from Current Approach

| Aspect | Current (Broken) | New (Blender Pattern) |
|--------|------------------|----------------------|
| Who starts first | Agent starts `graphite --mcp` | User starts Graphite |
| Editor location | Inside MCP process | Separate desktop app |
| MCP server | Heavy (full editor) | Lightweight (proxy only) |
| GUI | None (headless) | Full CEF window |
| User visibility | None | Real-time in Graphite window |
| Interactivity | Agent only | Agent + User simultaneously |

## Implementation Plan

### Phase 1: WebSocket Server in Graphite Desktop (Core)

**Goal:** Add a WebSocket server to the Graphite desktop app that accepts MCP tool calls.

**Files to create/modify:**
- `desktop/src/mcp_server.rs` — New module: WebSocket server + MCP handler
- `desktop/src/cli.rs` — Add `--mcp-server` flag (optional port)
- `desktop/src/app.rs` — Start MCP server when flag is set
- `desktop/Cargo.toml` — Add `tokio-tungstenite` dependency

**WebSocket Protocol:**
```json
// Client → Server: MCP tool call
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_rectangle","arguments":{...}}}

// Server → Client: MCP response
{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"Created rectangle"}]}}
```

**Message Flow:**
1. Client sends JSON-RPC via WebSocket
2. Server parses the message
3. Server dispatches to editor via `editor.handle_message(Message::...)`
4. Server collects `FrontendMessage` responses
5. Server returns JSON-RPC response to client

### Phase 2: Lightweight MCP Client

**Goal:** Create a thin MCP client that bridges stdio ↔ WebSocket.

**File to create:**
- `tools/graphite-mcp-client/src/main.rs` — Standalone binary

**Behavior:**
1. Connect to `ws://localhost:8080` (configurable via `GRAPHITE_WS_URL`)
2. Read JSON-RPC from stdin
3. Forward to WebSocket
4. Write response to stdout

**Dependencies:** Only `tokio`, `tokio-tungstenite`, `serde_json` — very lightweight.

### Phase 3: CLI Integration

**Graphite Desktop:**
```bash
# Start with MCP server on default port (8080)
graphite --mcp-server

# Start with MCP server on custom port
graphite --mcp-server --mcp-port 9090
```

**MCP Client config (Claude Desktop):**
```json
{
  "mcpServers": {
    "graphite": {
      "command": "/Users/glenn/tools/bin/graphite-mcp-client",
      "args": [],
      "env": {
        "GRAPHITE_WS_URL": "ws://localhost:8080"
      }
    }
  }
}
```

### Phase 4: Dev Server for Web Frontend (Optional)

For users who want the web interface instead of CEF:

```bash
# Terminal 1: Start Graphite with MCP server
graphite --mcp-server

# Terminal 2: Start Vite dev server
cd frontend && npm run dev

# Open http://localhost:5173 in browser
```

The Vite dev server proxies WebSocket connections to the Graphite backend.

## What Already Exists

| Component | Status | Notes |
|-----------|--------|-------|
| Graphite Desktop App | ✅ | Full GUI with CEF |
| Editor Message System | ✅ | `editor.handle_message()` |
| MCP Protocol Types | ✅ | `tools.rs`, `mcp_protocol.rs` |
| Node Catalog | ✅ | 272 nodes embedded |
| Tool Definitions | ✅ | 24 tools registered |
| WebSocket Server | ❌ | Needs implementation |
| MCP Client (proxy) | ❌ | Needs implementation |

## Estimated Effort

| Phase | Effort | Complexity |
|-------|--------|------------|
| Phase 1: WebSocket Server | 2-3 hours | Medium — need to integrate with winit event loop |
| Phase 2: MCP Client | 30 minutes | Low — simple stdio ↔ WebSocket bridge |
| Phase 3: CLI Integration | 30 minutes | Low — add flags and config |
| Phase 4: Dev Server | 1 hour | Low — Vite already configured |

**Total: ~4-5 hours**

## Open Questions

1. **Event loop integration:** The WebSocket server needs to run alongside the winit event loop. Options:
   - Spawn WebSocket on a separate thread, use channels to communicate with main thread
   - Use `tokio` runtime for WebSocket, `winit` for GUI (already done for node graph)

2. **Port conflicts:** What if port 8080 is taken? Auto-increment or fail?

3. **Authentication:** Should the WebSocket require a token? (Probably not for localhost)

4. **Multiple clients:** Can multiple MCP clients connect simultaneously? (Probably yes, but only one editor state)
