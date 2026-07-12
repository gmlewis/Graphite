# Graphite MCP Server

The MCP (Model Context Protocol) server for the Graphite vector graphics editor.
This is the entry point an AI agent (Claude Desktop, opencode, etc.) connects to
in order to drive the Graphite editor.

## Architecture

```
Agent (stdio JSON-RPC)
  │
  ▼
graphite-mcp-client        ← this binary, a thin stdio↔WebSocket relay
  │  (listens on ws://127.0.0.1:8081)
  ▼
Browser (Graphite web app running at http://localhost:8080)
  │  frontend/src/mcp-bridge.ts connects to the relay
  ▼
WASM editor                 ← frontend/wrapper/src/editor_wrapper.rs
   mcp_tool_call() dispatches into the real editor message system
```

The relay is deliberately tiny (~1 MB): it only forwards JSON-RPC lines from
stdin to the browser over WebSocket and pipes responses back to stdout. All
real work happens in the browser-side WASM editor, which is the same editor
the user sees and interacts with — so the agent and the user work on the same
document simultaneously, in real time.

## Setup

### 1. Start the Graphite web app

```bash
cd frontend && npm run dev -- --port 8080 --host 0.0.0.0
```

Then open <http://localhost:8080> in your browser. The browser tab must stay
open for the MCP server to function — closing it disconnects the agent.

### 2. Build and install the relay

```bash
./build-and-install.sh
# or just this crate:
cargo build --release -p graphite-mcp-client
cp target/release/graphite-mcp-client ~/tools/bin/
```

### 3. Configure your agent

Example for opencode (`~/.config/opencode/opencode.json`):

```json
{
  "mcp": {
    "graphite": {
      "type": "local",
      "command": ["graphite-mcp-client"],
      "enabled": true
    }
  }
}
```

The relay defaults to WebSocket port 8081. Override with the
`GRAPHITE_MCP_PORT` environment variable if needed.

## Requirements / Agent Guidance

These are the rules an agent **must** follow to avoid timeouts and errors:

1. **The browser must be connected.** The relay returns a `-32000` error
   ("Graphite web app is not connected") if no browser tab is open and
   registered with `mcp-bridge.ts`. If you see this, ask the user to open
   <http://localhost:8080> and wait a few seconds before retrying. Do **not**
   retry in a tight loop — that wastes the 15-second timeout budget.

2. **Tool calls are synchronous and have a 15-second timeout** (enforced in
   `graphite-mcp-client/src/main.rs`). Long operations will fail with error
   code `-32001`. Avoid issuing many tool calls in rapid parallel batches;
   prefer the batch tools (`batch_create`, `import_svg`) over hundreds of
   individual `create_rectangle` calls.

3. **Prefer batch tools.** `batch_create`, `import_svg`, and `create_path`
   are the efficient way to build complex artwork. Issuing hundreds of
   single-shape calls one-by-one is slow and will hit the timeout.

4. **`list_documents` only refreshes the UI document list** — it does *not*
   return a list of documents. To inspect the active document, use
   `get_document_info` instead.

5. **There is exactly one active document.** Tools operate on the active
   document. Use `create_document` to make a new one, then `get_document_info`
   to confirm it is active.

6. **Layer IDs are integers** (e.g. `42`), not GUID strings. They come from
   `get_layer_tree`, `get_selection`, or the `Created ...` responses of the
   create tools. Pass them as a number or a numeric string.

7. **`set_fill_color` sets fill OPACITY (0–100), not a color.** Despite the
   name, it dispatches `SetFillForSelectedLayers { fill: opacity/100 }`.
   There is currently no tool to set the fill *color* by hex value via this
   interface; use `import_svg` / `create_path` / `batch_create` with a
   `fill_color` field to create filled shapes. (Naming is a known wart —
   tracked separately.)

8. **`get_node_catalog` and `get_node_details` return a pointer message**
   when called through this relay, because the node catalog lives in the
   standalone `graphite-mcp-server` binary. They are listed here for
   completeness but are not the useful way to query nodes through the
   browser bridge.

## Tool reference (30 tools)

The authoritative tool list is defined in
`frontend/src/mcp-bridge.ts` (`TOOLS` array) and the handlers live in
`frontend/wrapper/src/editor_wrapper.rs` (`mcp_tool_handler`). If you change
one, change the other.

### Inspection (read-only)

| Tool | Args | Returns | Notes |
|------|------|---------|-------|
| `get_document_info` | — | name, layer/artboard/selection counts | Use this first to orient. |
| `get_layer_tree` | — | markdown list of all layers with IDs, kind, visibility, lock | Flat list, not nested. |
| `get_selection` | — | selected layer IDs + names | |
| `get_selected_layer_info` | — | per-layer bounds, visibility, lock | Richer than `get_selection`. |
| `get_layer_properties` | `layer_id` | name, kind, visible, locked | |
| `get_layer_bounds` | `layer_id` | min/max XY and size | |
| `get_node_graph` | `layer_id` | node implementation + inputs dump | |
| `list_documents` | — | `"Document list updated in UI"` | **Does NOT return a list** — use `get_document_info`. |

### Creation (write)

| Tool | Key args | Notes |
|------|----------|-------|
| `create_document` | `name` | Creates and switches to a new document. |
| `create_rectangle` | `x, y, width, height, fill_color, corner_radius` | SVG-backed; `fill_color` is hex. |
| `create_ellipse` | `x, y, radius_x, radius_y, fill_color` | `x,y` is the **center**. |
| `create_line` | `x1, y1, x2, y2, stroke_color, stroke_width` | |
| `create_text` | `x, y, text, font_size, fill_color` | Uses the default Graphite font. |
| `create_path` | `d, fill_color, stroke_color, stroke_width, x, y` | `d` is SVG path data. Most powerful primitive. |
| `import_svg` | `svg, name` | Full SVG string; placed at document origin. Supports `<rect>`, `<circle>`, `<path>`, `<g>`, `<linearGradient>`, … |
| `batch_create` | `shapes: [...]` | One call, many shapes. Each item: `{type, ...}` with `type ∈ {rectangle, ellipse, line, text}`. **Use this for complex drawings.** |

### Modification (operate on the current selection)

| Tool | Key args | Notes |
|------|----------|-------|
| `select_layer` | `layer_id` | Replaces the selection. |
| `delete_selected` | — | |
| `move_layer` | `dx, dy` | Nudges the current selection in document px. |
| `set_stroke` | `color, width` | Hex color; operates on the first selected layer. |
| `set_fill_color` | `opacity` (0–100) | **Sets fill OPACITY, not color** (see guidance above). |
| `set_opacity` | `opacity` (0–100) | Layer opacity. |
| `set_blend_mode` | `blend_mode` | One of the 25 BlendMode names (see enum below). |
| `activate_tool` | `tool` | `Select \| Pen \| Path \| Line \| Rectangle \| Ellipse \| Freehand \| Text \| Fill \| Gradient \| Eyedropper` |

### Viewport / history

| Tool | Key args | Notes |
|------|----------|-------|
| `zoom_to_fit` | — | Fit all content. |
| `set_viewport` | `zoom` | Zoom factor only (1.0 = 100%). |
| `undo` | — | |
| `redo` | — | |

### Node catalog (limited through the browser bridge)

| Tool | Args | Notes |
|------|------|-------|
| `get_node_catalog` | `category?, search?` | Returns a pointer message via the relay; use the standalone `graphite-mcp-server` binary for real catalog queries. |
| `get_node_details` | `node_id` | Same caveat. |

## Blend modes

`Normal`, `Darken`, `Multiply`, `ColorBurn`, `LinearBurn`, `DarkerColor`,
`Lighten`, `Screen`, `ColorDodge`, `LinearDodge`, `LighterColor`, `Overlay`,
`SoftLight`, `HardLight`, `VividLight`, `LinearLight`, `PinLight`, `HardMix`,
`Difference`, `Exclusion`, `Subtract`, `Divide`, `Hue`, `Saturation`,
`Color`, `Luminosity`.

## Tips for agents building complex artwork

- **Start with `get_document_info`** to confirm a document exists and is active.
  If not, call `create_document`.
- **Use `import_svg` for intricate vector art.** Construct one SVG string with
  many `<path>`, `<circle>`, `<rect>` elements and gradients; the WASM editor
  parses it in one shot. This is far faster and more reliable than issuing
  hundreds of individual `create_*` calls.
- **Use `batch_create`** when you need many simple shapes but don't want to
  hand-write SVG.
- **Read back what you built** with `get_layer_tree` / `get_selected_layer_info`
  before styling, so you have real layer IDs.
- **Don't parallel-fire tool calls.** The relay serializes stdin→WebSocket, but
  the browser handles them one at a time; parallel calls can interleave
  responses and starve the 15 s timeout. Issue calls sequentially.
- **After creating layers, call `zoom_to_fit`** so the user can see the result.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `-32000 Graphite web app is not connected` | Browser tab not open, or `mcp-bridge.ts` not connected to the relay. | Open <http://localhost:8080>; wait ~2 s. |
| `-32001 Tool call timed out` | Browser didn't respond in 15 s. | Reduce batch size; avoid parallel calls; check the browser console for errors. |
| `Editor error: …` modal text | The editor rejected the operation (e.g. no selection). | Read the message; select a layer first. |
| `Unknown tool: …` | Tool name typo, or the relay/browser version mismatch. | Check `TOOLS` in `frontend/src/mcp-bridge.ts`. |

## Building from source

```bash
cargo build --release -p graphite-mcp-client
```

The relay has no workspace deps beyond `tokio`, `tokio-tungstenite`, `serde`,
`serde_json`, `anyhow`, `futures-util` — it builds in seconds.

## Related files

| File | Role |
|------|------|
| `tools/graphite-mcp-client/src/main.rs` | The relay binary (this crate). |
| `frontend/src/mcp-bridge.ts` | Browser-side WebSocket client + tool list (`TOOLS`). |
| `frontend/wrapper/src/editor_wrapper.rs` | `mcp_tool_call` / `mcp_tool_handler` — the real tool implementations. |
| `tools/graphite-mcp-server/` | **Legacy** standalone crate for node-catalog-only queries. See its own README. |
| `Create-MCP-Server.md` | **Stale** historical implementation plan. Not authoritative. |