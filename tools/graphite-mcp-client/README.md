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

## Agent guidance: how to use these tools efficiently

These rules are derived from real-world usage. Following them prevents timeouts,
excessive layer counts, and failed operations.

### Workflow

1. **Start with `get_document_info`.** If it says "No active document", call
   `create_document`. Never call `list_documents` — it only refreshes the UI
   and returns no data.

2. **Plan your artwork as SVG.** Think in SVG elements (`<rect>`, `<circle>`,
   `<path>`, `<g>`, gradients). Build the SVG string and import it with one
   `import_svg` call. This creates a single layer group containing all
   elements — the most efficient approach.

3. **For complex curves** (mathematical curves, spirographs, flow fields,
   Lissajous figures), use `create_path` for individual curves or include
   `<path>` elements in your `import_svg` SVG. A single path can have hundreds
   of points and still create only one layer.

4. **Read back what you built** with `get_layer_tree` or `get_selected_layer_info`
   to get real layer IDs before calling `select_layer` / `set_stroke` / etc.

5. **Call `zoom_to_fit`** after creating content so the user can see the result.

### Tool efficiency hierarchy

```
import_svg     ← 1 call, 1 layer group, unlimited elements. BEST.
create_path    ← 1 call, 1 layer, 1 complex curve (hundreds of points OK).
batch_create   ← 1 call, N shapes, N layers. OK for ≤10 shapes.
create_*       ← 1 call, 1 shape, 1 layer. Fine for a few shapes only.
```

### Critical limitations

- **`batch_create` creates one layer per shape.** Each shape in the array is
  internally dispatched as a separate `InsertSvg` message, creating a separate
  layer. With 169 shapes, you get 169 layers. The browser slows down
  dramatically past ~200 layers, causing timeouts even on simple read calls.
  **Keep `batch_create` to ≤10 shapes per call**, and prefer `import_svg` for
  anything larger.

- **The browser slows down past ~200 layers.** If your document accumulates
  200+ layers (e.g. from many `batch_create` or `create_*` calls), even
  `get_document_info` may take longer. Start a new document with
  `create_document` rather than trying to delete layers one by one.

- **Tool call argument size is limited by the agent's context window.** The
  agent cannot inline more than ~20KB of data in a single tool-call parameter.
  A single `import_svg` call with a ~20KB SVG string works well. For larger
  artwork, split into multiple `import_svg` calls (each creates one layer
  group) or use `create_path` for individual complex curves.

- **Never issue tool calls in parallel.** The relay serializes stdin→WebSocket,
  but the browser handles them one at a time; parallel calls interleave
  responses and cause timeouts. Always call sequentially.

- **`set_fill_color` sets fill OPACITY (0–100), not a color.** Despite the name,
  it dispatches `SetFillForSelectedLayers { fill: opacity/100 }`. There is no
  tool to change the fill color of an existing layer — set `fill_color` in the
  SVG when creating the shape.

- **`set_blend_mode` always dispatches Normal.** This is a known implementation
  limitation (see `editor_wrapper.rs`).

- **`list_documents` is useless.** It returns `"Document list updated in UI"`
  — not a list. Use `get_document_info` instead.

- **`get_node_catalog` / `get_node_details` return a pointer message** through
  the browser relay. They require the standalone `graphite-mcp-server` binary.
  Do not call them for drawing tasks.

### Timeout behaviour

Tool calls have a **5-second timeout**. Every editor command should complete in
well under 1 second. The relay matches responses to requests by JSON-RPC `id`,
so a timed-out call cannot poison subsequent calls — its late reply is silently
discarded. If you see `-32001` errors, the browser is likely stuck (too many
layers) or disconnected.

### Layer IDs

Layer IDs are integers (e.g. `42`), not GUID strings. They come from
`get_layer_tree`, `get_selection`, `get_selected_layer_info`, or are implied by
the creation order. Pass them as a number or a numeric string.

## Tool reference (30 tools)

The authoritative tool list is defined in
`frontend/src/mcp-bridge.ts` (`TOOLS` array) and the handlers live in
`frontend/wrapper/src/editor_wrapper.rs` (`mcp_tool_handler`). If you change
one, change the other.

### Recommended workflow for complex artwork

The most efficient way to build complex artwork:

1. `get_document_info` — confirm a document exists
2. `create_document` — if needed
3. `import_svg` — import a single SVG with many elements (creates 1 layer group)
4. `create_path` — add individual complex curves if needed (1 layer each)
5. `zoom_to_fit` — show the user the result

**Avoid** `batch_create` and `create_*` for complex artwork — they create one
layer per shape and slow the browser down past ~200 layers. Use them only for
a handful of simple shapes.

### Inspection (read-only)

| Tool | Args | Returns | Notes |
|------|------|---------|-------|
| `get_document_info` | — | name, layer/artboard/selection counts | **Call this first.** Tells you if a document exists and how many layers it has. |
| `get_layer_tree` | — | markdown list of all layers with IDs, kind, visibility, lock | Flat list, not nested. Use to discover layer IDs. |
| `get_selection` | — | selected layer IDs + names | |
| `get_selected_layer_info` | — | per-layer bounds, visibility, lock | Richer than `get_selection` — includes bounding boxes. |
| `get_layer_properties` | `layer_id` | name, kind, visible, locked | |
| `get_layer_bounds` | `layer_id` | min/max XY and size | Useful for layout calculations. |
| `get_node_graph` | `layer_id` | node implementation + inputs dump | Advanced inspection. |
| `list_documents` | — | `"Document list updated in UI"` | **DEPRECATED — do not use.** Use `get_document_info`. |

### Creation (write)

| Tool | Key args | Layers created | Notes |
|------|----------|----------------|-------|
| `create_document` | `name` | 0 (empty doc) | Creates and switches to a new document. |
| `import_svg` | `svg, name` | **1 layer group** regardless of element count | **THE BEST TOOL for complex artwork.** Supports `<rect>`, `<circle>`, `<path>`, `<g>`, gradients, etc. Keep SVG under ~20KB per call. |
| `create_path` | `d, fill_color, stroke_color, stroke_width, x, y` | **1 layer** | Single complex curve. Hundreds of path points OK. |
| `batch_create` | `shapes: [...]` | **N layers** (one per shape) | **≤10 shapes per call.** Each shape is a separate InsertSvg internally. Prefer `import_svg` for anything larger. |
| `create_rectangle` | `x, y, width, height, fill_color, corner_radius` | 1 | For a few shapes only. |
| `create_ellipse` | `x, y, radius_x, radius_y, fill_color` | 1 | `x,y` is the **center**. |
| `create_line` | `x1, y1, x2, y2, stroke_color, stroke_width` | 1 | |
| `create_text` | `x, y, text, font_size, fill_color` | 1 | Default Graphite font. |

### Modification (operate on the current selection)

| Tool | Key args | Notes |
|------|----------|-------|
| `select_layer` | `layer_id` | Required before any style/transform tool. Replaces selection. |
| `delete_selected` | — | |
| `move_layer` | `dx, dy` | Nudges selection in document px. |
| `set_stroke` | `color, width` | Hex color; first selected layer only. |
| `set_fill_color` | `opacity` (0–100) | **Sets fill OPACITY, not color.** |
| `set_opacity` | `opacity` (0–100) | Layer opacity. |
| `set_blend_mode` | `blend_mode` | **Always dispatches Normal** (known limitation). |
| `activate_tool` | `tool` | Changes active tool in UI. Does not simulate drawing. |

### Viewport / history

| Tool | Key args | Notes |
|------|----------|-------|
| `zoom_to_fit` | — | **Always call after creating content.** |
| `set_viewport` | `zoom` | Zoom factor only (1.0 = 100%). No pan. |
| `undo` | — | |
| `redo` | — | |

### Node catalog (not useful through the browser relay)

| Tool | Args | Notes |
|------|------|-------|
| `get_node_catalog` | `category?, search?` | Returns a pointer message via the relay. Requires standalone `graphite-mcp-server` binary. **Do not call for drawing tasks.** |
| `get_node_details` | `node_id` | Same caveat. **Do not call for drawing tasks.** |

## Blend modes

`Normal`, `Darken`, `Multiply`, `ColorBurn`, `LinearBurn`, `DarkerColor`,
`Lighten`, `Screen`, `ColorDodge`, `LinearDodge`, `LighterColor`, `Overlay`,
`SoftLight`, `HardLight`, `VividLight`, `LinearLight`, `PinLight`, `HardMix`,
`Difference`, `Exclusion`, `Subtract`, `Divide`, `Hue`, `Saturation`,
`Color`, `Luminosity`.

## Example: building complex generative artwork

Here is the recommended pattern for building artwork that would be impossible
to draw by hand (e.g. a flow-field painting with 150 curved paths, 40 gradient
rings, mathematical rose curves, epicycloids, and Lissajous overlays):

```
1. get_document_info              → "No active document"
2. create_document { name: "..." } → "Document created"
3. import_svg { svg: "<svg>...50 flow-field <path> elements + 30 <circle> rings...</svg>" }
   → "SVG imported successfully"   (1 layer group, ~50 paths + 30 circles)
4. create_path { d: "M...rose curve k=5/3...", stroke_color: "#ffe066" }
   → "Path created"                (1 layer)
5. create_path { d: "M...epicycloid...", stroke_color: "#ff00aa" }
   → "Path created"                (1 layer)
6. zoom_to_fit                     → "Zoomed to fit"
```

Total: 6 tool calls, ~4 layers, hundreds of elements. The key insight: put as
much as possible into the `import_svg` SVG string so it all becomes one layer
group. Use `create_path` only for individual complex curves that are easier to
express as standalone path data.

**What NOT to do:** calling `batch_create` with 169 rectangle shapes creates
169 separate layers and slows the browser to a crawl. Use `import_svg` with
169 `<rect>` elements instead — same visual result, but only 1 layer.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `-32000 Graphite web app is not connected` | Browser tab not open, or `mcp-bridge.ts` not connected to the relay. | Open <http://localhost:8080>; wait ~2 s. |
| `-32001 Tool call timed out` | Browser didn't respond in 5 s. Usually means too many layers (>200) or browser is stuck. | Check the browser tab is alive. If the document has >200 layers, create a new document. Reduce `batch_create` batch size to ≤10. Avoid parallel calls. |
| `Editor error: …` modal text | The editor rejected the operation (e.g. no selection). | Read the message; call `select_layer` with a valid ID first. |
| `Unknown tool: …` | Tool name typo, or the relay/browser version mismatch. | Check `TOOLS` in `frontend/src/mcp-bridge.ts`. |
| Tool calls used to work but now all time out | Document has accumulated too many layers and the browser is bogged down. | Call `create_document` to start fresh, or ask the user to reload the browser tab. |
| `batch_create` works for small batches but times out for large ones | Each shape creates a separate layer; the browser slows with many layers. | Use `import_svg` instead — 1 layer group regardless of element count. |

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