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

### Phase 3: Wire Editor Bridge 🔲
- Connect tool handlers to `Editor::handle_message()` dispatching
- Implement `Editor` initialization in headless mode
- Wire `FrontendMessage` responses back to tool results
- Handle async node graph evaluation via `poll_node_graph_evaluation()`

### Phase 4: Expand Tool Set 🔲
- Add remaining tools from the full tool set list
- Add parameter validation using node catalog
- Add error handling and meaningful error messages

### Phase 5: Integration & Testing 🔲
- Add MCP server launch option to Graphite CLI
- Test with Claude/other MCP clients
- Document tool usage

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

**Layer 2: MCP Server (Rust, in the editor crate)**
- Uses `rmcp` or `tower-lsp` crate for MCP protocol
- Transport: WebSocket or stdio
- Each tool handler dispatches into the existing `GraphiteEditor` message system via `editor_handle.process_message(...)`
- The MCP server is just another "frontend" — same path the web frontend uses

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

### Phase 3: Core Tools (Proof of Concept)
- Implement first 5-6 tools:
  - `get_document_info`
  - `get_layer_tree`
  - `create_layer` (rectangle)
  - `select_layer`
  - `set_fill`
  - `add_node` + `connect_nodes`
- Wire each tool to existing `EditorMessage` / `NodeGraphMessage` dispatches
- **Key insight:** Each tool handler calls `editor.process_message(EditorMessage::...())` — identical to how the frontend works

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
