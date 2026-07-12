# Graphite Editor MCP Server — Implementation Plan

## Progress

### Phase 1: Node Catalog Generator ✅
- **Script:** `tools/node_catalog_generator.py`
- **Output:** `node_catalog.json` (272 nodes, 31 categories)
- Parses all `#[node_macro::node]` annotated functions from 57 Rust source files
- Extracts: name, node_id, category, description, inputs (name, type, default, constraints, implementations), output type, async status, properties, shader_node
- Run: `python3 tools/node_catalog_generator.py` (regenerates `node_catalog.json`)

### Phase 2: MCP Server Scaffold ✅
- **Crate:** `tools/graphite-mcp-server/` (standalone binary, no workspace deps needed)
- **Binary:** `graphite-mcp`
- JSON-RPC 2.0 over stdio transport (MCP protocol)
- 24 tools registered (see tool list below)
- Node catalog embedded at compile time via `include_str!` — binary is self-contained
- Editor bridge stubs ready for wiring to `Editor::handle_message()`
- Build: `cd tools/graphite-mcp-server && cargo build` (requires Rust 1.88+ for workspace, or standalone with `rustup run 1.95.0 cargo build`)

### Phase 3: Wire Editor Bridge 🔄 (headed-first)
- **Architecture decision: headed-first** — MCP server runs in-process within a running editor
- Editor launched with `--mcp` flag starts MCP server on stdin/stdout alongside GUI
- MCP server reads JSON-RPC from stdin, dispatches tool calls via `AppEvent::McpToolCall` to main thread
- Main thread processes tool calls via `DesktopWrapperMessage::FromWeb(Message::...)` dispatch
- `FrontendMessage` responses converted to MCP tool results
- Headless mode (standalone binary, no GUI) for catalog-only queries via `--standalone` flag
- **Working tools (dispatched to editor):** `list_documents`, `create_document`, `delete_selected`, `undo`, `redo`, `activate_tool`, `zoom_to_fit`
- **Working tools (catalog-only, no editor needed):** `get_node_catalog`, `get_node_details`
- **Stub tools (return placeholder):** `get_layer_tree`, `create_rectangle/ellipse/line/text`, `select_layer`, `set_fill_color`, `set_stroke`, `set_opacity`, `set_blend_mode`, `move_layer`, `get_selection`, `get_layer_properties`, `get_node_graph`, `set_viewport`
- **Files modified:** `editor/Cargo.toml` (added `headless` feature), `editor/src/application.rs`, `editor/src/node_graph_executor.rs`, `editor/src/node_graph_executor/runtime_io.rs`, `desktop/Cargo.toml`, `desktop/src/cli.rs`, `desktop/src/lib.rs`, `desktop/src/app.rs`, `desktop/src/event.rs`, `desktop/wrapper/Cargo.toml`
- **Files created:** `desktop/src/mcp.rs`, `tools/graphite-mcp-server/src/lib.rs`
- **Key insight:** MCP thread reads stdin → sends `AppEvent::McpToolCall` with oneshot response channel → main thread dispatches to editor → response sent back via channel

### Phase 4: Expand Tool Set ✅
- Implemented all message-dispatch tools:
  - **create_rectangle/ellipse/line/text** — generates SVG and dispatches `DocumentMessage::InsertSvg`
  - **select_layer** — dispatches `DocumentMessage::SelectLayer { id, ctrl, shift }`
  - **set_fill_color** — dispatches `DocumentMessage::SetFillForSelectedLayers { fill }`
  - **set_opacity** — dispatches `DocumentMessage::SetOpacityForSelectedLayers { opacity }`
  - **set_blend_mode** — dispatches `DocumentMessage::SetBlendModeForSelectedLayers { blend_mode }` with full enum mapping (27 modes)
  - **move_layer** — dispatches `DocumentMessage::NudgeSelectedLayers { delta_x, delta_y, resize, resize_opposite }`
  - **set_viewport** — dispatches `NavigationMessage::CanvasZoomSet { zoom_factor }` (zoom only)
- Implemented state-reading tools via direct `Editor` access:
  - **get_layer_tree** — iterates `metadata.all_layers()`, reads name/visible/locked from `network_interface`
  - **get_selection** — reads `selected_nodes().selected_layers(metadata)` with layer names
  - **get_layer_properties** — reads name, kind, visible, locked for a specific layer by ID
  - **get_node_graph** — reads `document_node()` implementation and inputs for a layer
  - **set_stroke** — parses hex color, dispatches `GraphOperationMessage::StrokeSet` on first selected layer
- Added public `editor()` accessor to `DesktopWrapper` and `active_document()` / `active_document_mut()` to `Editor`
- Added `graphene-std` and `graph-craft` dependencies to desktop crate (behind `mcp` feature)
- **Total working tools: 24** (all registered tools now functional)

### Phase 5: Integration & Testing ✅
- ✅ MCP server launch option added to Graphite CLI (`--mcp` flag)
- ✅ Claude MCP configuration file created (`claude_mcp_config.json`)
- ✅ MCP client test script created and verified (`test_mcp.sh`)
- ✅ Comprehensive documentation with all 24 tools and examples (`README.md`)
- ✅ Build script for release builds and installation (`build-and-install.sh`)
- **Test results:** Initialize, tools/list (24 tools), get_node_catalog (272 nodes), get_node_details all working

## Goal

Build a full-featured MCP (Model Context Protocol) Server for the Graphite Editor that gives any AI agent 100% control over the editor. The server reuses Graphite's existing message-passing architecture rather than building a parallel API.

## Architecture

### Two-Layer Design

**Layer 1: Node Catalog Generator (script)**
- Parses all `#[node_macro::node]` annotated Rust functions across the codebase
- Extracts: name, category, inputs (param name + type + default), outputs (return type), widget hints
- Outputs `node_catalog.json` — machine-readable manifest of every node
- This catalog is loaded by the MCP server at startup for tool descriptions and parameter validation
- **Not** a 1:1 tool mapping — it's a data source

**Layer 2: MCP Server (Rust, embedded in desktop app)**
- Runs as a dedicated thread within the Graphite desktop app (when launched with `--mcp`)
- Reads JSON-RPC 2.0 messages from stdin, writes responses to stdout
- Tool calls dispatched to the main thread via `AppEvent::McpToolCall` with a oneshot response channel
- Main thread processes the tool call and dispatches into the editor via `DesktopWrapperMessage::FromWeb(Message::...)`
- The MCP server is just another "frontend" — same path the web frontend uses
- Also available as standalone binary (`graphite-mcp --standalone`) for catalog-only queries

### Tool Set (~30-50 high-level tools)

#### Node Catalog Tools (working, data from embedded catalog)
- `get_node_catalog` — list/search all 272 nodes with categories and descriptions
- `get_node_details` — full details for a node: inputs, outputs, defaults, constraints, types

#### Document Tools (stubs)
- `create_document` — new blank document with dimensions
- `list_documents` — list open documents

#### Layer Tools (stubs)
- `get_layer_tree` — full layer hierarchy with IDs
- `create_rectangle` — rectangle shape with position, size, fill, corner radius
- `create_ellipse` — ellipse shape with center, radii, fill
- `create_line` — line with endpoints, stroke color/width
- `create_text` — text layer with position, content, font size, color
- `select_layer` — select by ID
- `delete_selected` — delete selected layers
- `get_selection` — list selected layer IDs
- `get_layer_properties` — all properties of a layer

#### Style Operations
- `set_fill` — solid color, gradient, etc
- `set_stroke` — color, width, dash pattern
- `set_blend_mode`
- `set_opacity`
- `get_layer_style` — read current style attributes

#### Transform Operations
- `move_layer` — translate by dx/dy
- `scale_layer` — scale by factor
- `rotate_layer` — rotate by degrees
- `get_layer_transform` — read current transform

#### Selection Operations
- `select_at_position` — click at canvas x,y
- `select_by_id`
- `get_selection` — list selected layer IDs

#### Tool Operations
- `activate_tool` — pen, rectangle, ellipse, text, line, select, etc
- `draw_with_tool` — simulate tool input (for programmatic drawing)

#### Viewport Operations
- `zoom_to_fit` — fit all layers in view
- `zoom_to_selection`
- `set_viewport` — pan/zoom to specific position
- `get_canvas_state` — current zoom, pan offset

#### Text Operations
- `set_text_content`
- `set_font` / `set_font_size`
- `get_text_content`

#### Raster/Image Operations
- `import_image`
- `apply_filter` — blur, sharpen, etc
- `adjust色彩` — brightness, contrast, hue/saturation

#### Inspection Operations
- `get_layer_properties` — all attributes of a layer
- `get_node_catalog` — list available nodes with descriptions
- `screenshot` — render current canvas to image

## Implementation Phases

### Phase 1: Node Catalog Generator
- Write a Python or Rust script (~300 lines)
- Parse files in `node-graph/nodes/*/src/*.rs`
- Parse `node-graph/libraries/core-types/src/registry.rs`
- Extract from `#[node_macro::node]` annotations and function signatures
- Output `node_catalog.json`
- **Files to create:** `tools/node_catalog_generator.py` (or `.rs`)

### Phase 2: MCP Server Scaffold
- Add MCP server as a new crate or module in the editor
- Set up `rmcp` dependency
- Implement stdio and/or WebSocket transport
- Register basic tool dispatcher
- **Files to create:** `editor/src/mcp/` directory with `mod.rs`, `server.rs`, `tools.rs`

### Phase 3: Core Tools (Proof of Concept) ✅
- Implemented first batch of tools wired to the editor:
  - `list_documents` — dispatches `PortfolioMessage::UpdateOpenDocumentsList`
  - `create_document` — dispatches `PortfolioMessage::NewDocumentWithName`
  - `delete_selected` — dispatches `DocumentMessage::DeleteSelectedLayers`
  - `undo`/`redo` — dispatches `DocumentHistoryBackward`/`Forward`
  - `activate_tool` — dispatches `ToolMessage::ActivateTool`
  - `zoom_to_fit` — dispatches `DocumentMessage::ZoomCanvasToFitAll`
  - `get_node_catalog`/`get_node_details` — catalog-only, no editor needed
- Added `--mcp` flag to desktop CLI for headed mode
- Added `--standalone` flag to MCP binary for catalog-only mode
- MCP thread communicates with main thread via `mpsc` channels
- **Key insight:** Each tool handler sends `DesktopWrapperMessage::FromWeb(Box::new(Message::...))` — identical to how the web frontend works
- **Files modified:** `editor/Cargo.toml`, `editor/src/application.rs`, `editor/src/node_graph_executor.rs`, `editor/src/node_graph_executor/runtime_io.rs`, `desktop/Cargo.toml`, `desktop/src/cli.rs`, `desktop/src/lib.rs`, `desktop/src/app.rs`, `desktop/src/event.rs`, `desktop/wrapper/Cargo.toml`, `tools/graphite-mcp-server/Cargo.toml`, `tools/graphite-mcp-server/src/lib.rs`, `tools/graphite-mcp-server/src/main.rs`, `tools/graphite-mcp-server/src/tools.rs`, `tools/graphite-mcp-server/src/editor_bridge.rs`
- **Files created:** `desktop/src/mcp.rs`

### Phase 4: Expand Tool Set
- Add remaining tools from the tool set list above
- Add parameter validation using node catalog
- Add error handling and meaningful error messages

### Phase 5: Integration & Testing
- Add MCP server launch option to Graphite CLI
- Test with Claude/other MCP clients
- Document tool usage in README

## Key Technical Decisions

1. **Rust implementation** — stay in the same language as the editor, directly use internal types and message system
2. **Reuse message infrastructure** — the MCP server dispatches `EditorMessage`, `NodeGraphMessage`, `DocumentMessage` etc. just like the frontend does
3. **Node catalog as JSON** — generated artifact, checked into repo or generated at build time
4. **Tool granularity** — high-level semantic tools (not 1:1 node mappings). Agents want "add blur to layer", not "create blur node, wire inputs, set parameters"
5. **Transport** — start with stdio (simplest for MCP), add WebSocket later for remote access
6. **Headed-first architecture** — MCP server runs in-process within a running editor (not headless). This gives AI agents visual feedback via screenshots and works with the user's existing session. The MCP thread reads stdin and sends `AppEvent::McpToolCall` to the main thread via a channel, which dispatches to the editor. Headless mode is available via `--standalone` flag for catalog-only queries.
7. **Feature-gated MCP** — MCP support is behind `--features mcp` in the desktop crate. The `headless` feature on `graphite-editor` exposes internal APIs needed for the standalone mode.

## Reference Files

### Node Definition Locations (for catalog generator)
- `node-graph/nodes/math/src/lib.rs` — math, logic, color, value, vector math
- `node-graph/nodes/vector/src/vector_nodes.rs` — vector style, modifier, measure
- `node-graph/nodes/vector/src/generator_nodes.rs` — shape generators
- `node-graph/nodes/vector/src/vector_modification_nodes.rs` — internal vector mods
- `node-graph/nodes/raster/src/adjustments.rs` — image adjustments
- `node-graph/nodes/raster/src/filter.rs` — blur, dehaze
- `node-graph/nodes/raster/src/gradient_map.rs` — gradient map
- `node-graph/nodes/raster/src/blending_nodes.rs` — composite, color overlay
- `node-graph/nodes/raster/src/std_nodes.rs` — imaginate, noise, mosaic
- `node-graph/nodes/raster/src/dehaze.rs` — dehaze
- `node-graph/nodes/raster/src/image_color_palette.rs` — color palette
- `node-graph/nodes/text/src/lib.rs` — text manipulation
- `node-graph/nodes/text/src/regex.rs` — regex operations
- `node-graph/nodes/text/src/json.rs` — JSON operations
- `node-graph/nodes/transform/src/transform_nodes.rs` — transforms
- `node-graph/nodes/graphic/src/graphic.rs` — layer, attributes, rasterize
- `node-graph/nodes/graphic/src/artboard.rs` — artboard
- `node-graph/nodes/repeat/src/repeat_nodes.rs` — repeat, for-each
- `node-graph/nodes/blending/src/lib.rs` — blend modes
- `node-graph/nodes/gcore/src/context.rs` — context reads
- `node-graph/nodes/gcore/src/animation.rs` — animation
- `node-graph/nodes/gcore/src/debug.rs` — debug nodes
- `node-graph/nodes/gcore/src/ops.rs` — core ops
- `node-graph/nodes/gcore/src/memo.rs` — memoize
- `node-graph/nodes/gcore/src/extract_xy.rs` — extract XY
- `node-graph/nodes/gstd/src/text.rs` — text to graphic/vector
- `node-graph/nodes/gstd/src/platform_application_io.rs` — HTTP, resources
- `node-graph/nodes/path-bool/src/lib.rs` — path boolean
- `node-graph/nodes/brush/src/brush.rs` — brush

### MCP Protocol Infrastructure
- `node-graph/node-macro/src/parsing.rs` — parses `#[node_macro::node]`
- `node-graph/node-macro/src/codegen.rs` — generates Node impl
- `node-graph/libraries/core-types/src/registry.rs` — NodeMetadata, FieldMetadata, NODE_REGISTRY, NODE_METADATA

### Editor Message System (where tools dispatch into)
- `editor/src/messages/` — all message types (EditorMessage, DocumentMessage, NodeGraphMessage, etc.)
- `editor/src/messages/portfolio/document/node_graph/document_node_definitions.rs` — document node definitions
- `editor/src/messages/portfolio/document/node_graph/document_node_definitions/document_node_derive.rs` — post_process_nodes()
