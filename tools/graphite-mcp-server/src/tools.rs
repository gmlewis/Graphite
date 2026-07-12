use serde_json::Value;
use crate::mcp_protocol::{Tool, ToolContent};

pub fn list_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "get_node_catalog".into(),
            description: "List all available nodes with their categories, inputs, outputs, and parameters. Returns the full node catalog.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Optional: filter by category name"
                    },
                    "search": {
                        "type": "string",
                        "description": "Optional: search nodes by name or description"
                    }
                }
            }),
        },
        Tool {
            name: "get_node_details".into(),
            description: "Get detailed information about a specific node by its ID, including all inputs, outputs, defaults, and valid type implementations.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node function name (e.g. 'blur', 'circle', 'fill')"
                    }
                },
                "required": ["node_id"]
            }),
        },
        Tool {
            name: "list_documents".into(),
            description: "List all open documents with their IDs, names, and metadata.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "get_layer_tree".into(),
            description: "Get the full layer tree of a document, showing the hierarchy of layers, groups, and artboards with their IDs, names, visibility, and lock state.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "document_id": {
                        "type": "string",
                        "description": "The document ID. If omitted, uses the active document."
                    }
                }
            }),
        },
        Tool {
            name: "create_document".into(),
            description: "Create a new blank document with specified dimensions.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Document name",
                        "default": "Untitled"
                    },
                    "width": {
                        "type": "number",
                        "description": "Document width in pixels",
                        "default": 1920
                    },
                    "height": {
                        "type": "number",
                        "description": "Document height in pixels",
                        "default": 1080
                    }
                }
            }),
        },
        Tool {
            name: "create_rectangle".into(),
            description: "Create a rectangle shape layer in the active document.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "X position" },
                    "y": { "type": "number", "description": "Y position" },
                    "width": { "type": "number", "description": "Width" },
                    "height": { "type": "number", "description": "Height" },
                    "fill_color": {
                        "type": "string",
                        "description": "Fill color as hex (e.g. '#FF0000') or name (e.g. 'red')",
                        "default": "#000000"
                    },
                    "corner_radius": {
                        "type": "number",
                        "description": "Corner radius in pixels",
                        "default": 0
                    }
                },
                "required": ["x", "y", "width", "height"]
            }),
        },
        Tool {
            name: "create_ellipse".into(),
            description: "Create an ellipse shape layer in the active document.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "Center X position" },
                    "y": { "type": "number", "description": "Center Y position" },
                    "radius_x": { "type": "number", "description": "Horizontal radius" },
                    "radius_y": { "type": "number", "description": "Vertical radius" },
                    "fill_color": {
                        "type": "string",
                        "description": "Fill color as hex or name",
                        "default": "#000000"
                    }
                },
                "required": ["x", "y", "radius_x", "radius_y"]
            }),
        },
        Tool {
            name: "create_line".into(),
            description: "Create a line shape layer in the active document.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "x1": { "type": "number", "description": "Start X" },
                    "y1": { "type": "number", "description": "Start Y" },
                    "x2": { "type": "number", "description": "End X" },
                    "y2": { "type": "number", "description": "End Y" },
                    "stroke_color": {
                        "type": "string",
                        "description": "Stroke color as hex or name",
                        "default": "#000000"
                    },
                    "stroke_width": {
                        "type": "number",
                        "description": "Stroke width in pixels",
                        "default": 2
                    }
                },
                "required": ["x1", "y1", "x2", "y2"]
            }),
        },
        Tool {
            name: "create_text".into(),
            description: "Create a text layer in the active document.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "X position" },
                    "y": { "type": "number", "description": "Y position" },
                    "text": { "type": "string", "description": "Text content" },
                    "font_size": {
                        "type": "number",
                        "description": "Font size in pixels",
                        "default": 24
                    },
                    "fill_color": {
                        "type": "string",
                        "description": "Text color as hex or name",
                        "default": "#000000"
                    }
                },
                "required": ["x", "y", "text"]
            }),
        },
        Tool {
            name: "select_layer".into(),
            description: "Select a layer by its ID.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "layer_id": { "type": "string", "description": "The layer ID to select" },
                    "clear_existing": {
                        "type": "boolean",
                        "description": "Whether to clear the current selection first",
                        "default": true
                    }
                },
                "required": ["layer_id"]
            }),
        },
        Tool {
            name: "delete_selected".into(),
            description: "Delete all currently selected layers.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "undo".into(),
            description: "Undo the last action.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "redo".into(),
            description: "Redo the last undone action.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "set_fill_color".into(),
            description: "Set the fill color of selected layers.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "color": {
                        "type": "string",
                        "description": "Color as hex (e.g. '#FF0000') or name (e.g. 'red', 'blue')"
                    }
                },
                "required": ["color"]
            }),
        },
        Tool {
            name: "set_stroke".into(),
            description: "Set the stroke properties of selected layers.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "color": {
                        "type": "string",
                        "description": "Stroke color as hex or name"
                    },
                    "width": {
                        "type": "number",
                        "description": "Stroke width in pixels"
                    }
                }
            }),
        },
        Tool {
            name: "set_opacity".into(),
            description: "Set the opacity of selected layers.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "opacity": {
                        "type": "number",
                        "description": "Opacity percentage (0-100)",
                        "minimum": 0,
                        "maximum": 100
                    }
                },
                "required": ["opacity"]
            }),
        },
        Tool {
            name: "set_blend_mode".into(),
            description: "Set the blend mode of selected layers.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "blend_mode": {
                        "type": "string",
                        "description": "Blend mode name",
                        "enum": ["Normal", "Darken", "Multiply", "ColorBurn", "LinearBurn", "DarkerColor", "Lighten", "Screen", "ColorDodge", "LinearDodge", "LighterColor", "Overlay", "SoftLight", "HardLight", "VividLight", "LinearLight", "PinLight", "HardMix", "Difference", "Exclusion", "Subtract", "Divide", "Hue", "Saturation", "Color", "Luminosity"]
                    }
                },
                "required": ["blend_mode"]
            }),
        },
        Tool {
            name: "move_layer".into(),
            description: "Move selected layers by a delta offset.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "dx": { "type": "number", "description": "Horizontal offset in pixels" },
                    "dy": { "type": "number", "description": "Vertical offset in pixels" }
                },
                "required": ["dx", "dy"]
            }),
        },
        Tool {
            name: "get_selection".into(),
            description: "Get the list of currently selected layer IDs.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "get_layer_properties".into(),
            description: "Get all properties of a specific layer including its transform, style, and node graph.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "layer_id": { "type": "string", "description": "The layer ID" }
                },
                "required": ["layer_id"]
            }),
        },
        Tool {
            name: "get_node_graph".into(),
            description: "Get the node graph of a specific layer, showing all nodes and their connections.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "layer_id": { "type": "string", "description": "The layer ID" }
                },
                "required": ["layer_id"]
            }),
        },
        Tool {
            name: "activate_tool".into(),
            description: "Activate a tool (e.g., Select, Pen, Rectangle, Ellipse, Line, Text, Fill, Gradient, Eyedropper).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tool": {
                        "type": "string",
                        "description": "Tool name to activate",
                        "enum": ["Select", "Pen", "Path", "Line", "Rectangle", "Ellipse", "Polygon", "Star", "Spiral", "Freehand", "Text", "Fill", "Gradient", "Eyedropper", "NdTree", "Trim", "BooleanOperation"]
                    }
                },
                "required": ["tool"]
            }),
        },
        Tool {
            name: "zoom_to_fit".into(),
            description: "Zoom the viewport to fit all content.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "set_viewport".into(),
            description: "Set the viewport position and zoom level.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "Viewport center X" },
                    "y": { "type": "number", "description": "Viewport center Y" },
                    "zoom": { "type": "number", "description": "Zoom level (1.0 = 100%)" }
                }
            }),
        },
    ]
}

pub fn call_tool(name: &str, args: Value) -> anyhow::Result<Vec<ToolContent>> {
    match name {
        "get_node_catalog" => get_node_catalog(&args),
        "get_node_details" => get_node_details(&args),
        _ => Err(anyhow::anyhow!("Unknown catalog tool: {}", name)),
    }
}

pub async fn call_tool_async(name: &str, args: Value) -> anyhow::Result<Vec<ToolContent>> {
    // Delegate to the editor bridge
    crate::editor_bridge::dispatch_tool_call(name, args).await
}

// Embed the node catalog at compile time
const CATALOG_JSON: &str = include_str!("../../../node_catalog.json");

fn get_catalog() -> anyhow::Result<&'static serde_json::Value> {
    use std::sync::OnceLock;
    static CATALOG: OnceLock<serde_json::Value> = OnceLock::new();
    if let Some(c) = CATALOG.get() {
        return Ok(c);
    }
    let catalog: serde_json::Value = serde_json::from_str(CATALOG_JSON)?;
    let _ = CATALOG.set(catalog);
    Ok(CATALOG.get().unwrap())
}

fn get_node_catalog(args: &Value) -> anyhow::Result<Vec<ToolContent>> {
    let catalog = get_catalog()?;
    let category_filter = args.get("category").and_then(|v| v.as_str());
    let search_filter = args.get("search").and_then(|v| v.as_str());

    let mut output = String::new();
    let total = catalog["total_nodes"].as_u64().unwrap_or(0);
    let num_cats = catalog["categories"].as_object().map(|o| o.len()).unwrap_or(0);
    output.push_str(&format!("Graphite Node Catalog: {total} nodes in {num_cats} categories\n\n"));

    let categories = catalog["categories"].as_object().ok_or_else(|| anyhow::anyhow!("Invalid catalog"))?;
    for (cat_name, cat_data) in categories {
        if let Some(filter) = category_filter {
            if !cat_name.to_lowercase().contains(&filter.to_lowercase()) {
                continue;
            }
        }
        let empty_vec = vec![];
        let nodes = cat_data["nodes"].as_array().unwrap_or(&empty_vec);
        output.push_str(&format!("## {cat_name} ({} nodes)\n", nodes.len()));

        for node in nodes {
            let node_name = node["name"].as_str().unwrap_or("?");
            let desc = node["description"].as_str().unwrap_or("");
            let node_id = node["node_id"].as_str().unwrap_or("?");

            if let Some(search) = search_filter {
                let s = search.to_lowercase();
                if !node_name.to_lowercase().contains(&s)
                    && !desc.to_lowercase().contains(&s)
                    && !node_id.to_lowercase().contains(&s)
                {
                    continue;
                }
            }

            let inputs = node["inputs"].as_array().map(|a| a.len()).unwrap_or(0);
            let first_desc = desc.lines().next().unwrap_or("");
            output.push_str(&format!("  - **{node_name}** (`{node_id}`): {first_desc} [{inputs} inputs]\n"));
        }
        output.push('\n');
    }
    Ok(vec![ToolContent::Text { text: output }])
}

fn get_node_details(args: &Value) -> anyhow::Result<Vec<ToolContent>> {
    let node_id = args.get("node_id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: node_id"))?;
    let catalog = get_catalog()?;
    let categories = catalog["categories"].as_object().ok_or_else(|| anyhow::anyhow!("Invalid catalog"))?;

    for (_cat_name, cat_data) in categories {
        let empty_vec = vec![];
        for node in cat_data["nodes"].as_array().unwrap_or(&empty_vec) {
            if node["node_id"].as_str() == Some(node_id) {
                let mut out = String::new();
                out.push_str(&format!("# {}\n", node["name"].as_str().unwrap_or("")));
                out.push_str(&format!("**Category:** {}\n", node["category"].as_str().unwrap_or("")));
                out.push_str(&format!("**ID:** `{node_id}`\n"));
                out.push_str(&format!("**File:** {}:{}\n", node["file"].as_str().unwrap_or(""), node["line"].as_u64().unwrap_or(0)));
                if let Some(desc) = node["description"].as_str().filter(|d| !d.is_empty()) {
                    out.push_str(&format!("\n{desc}\n"));
                }
                if node["async_fn"].as_bool() == Some(true) {
                    out.push_str("\n*This node is asynchronous.*\n");
                }

                out.push_str("\n## Inputs\n");
                if let Some(inputs) = node["inputs"].as_array() {
                    for inp in inputs {
                        let name = inp["name"].as_str().unwrap_or("?");
                        let ty = inp["rust_type"].as_str().unwrap_or("?");
                        let desc = inp["description"].as_str().unwrap_or("");
                        let default = inp.get("default").filter(|v| !v.is_null()).map(|v| format!(" = {v}")).unwrap_or_default();
                        let hidden = if inp["hidden"].as_bool() == Some(true) { " [hidden]" } else { "" };
                        let exposed = if inp["exposed"].as_bool() == Some(true) { " [exposed]" } else { "" };
                        let gpu = if inp["gpu_image"].as_bool() == Some(true) { " [GPU]" } else { "" };
                        out.push_str(&format!("- **{name}**: `{ty}{default}`{hidden}{exposed}{gpu}"));
                        if !desc.is_empty() {
                            out.push_str(&format!(" — {}", desc.lines().next().unwrap_or("")));
                        }
                        out.push('\n');

                        let mut constraints = vec![];
                        if inp["range"].as_bool() == Some(true) { constraints.push("range".into()); }
                        if let (Some(lo), Some(hi)) = (inp["soft_min"].as_f64(), inp["soft_max"].as_f64()) {
                            constraints.push(format!("soft: {lo}..{hi}"));
                        }
                        if let (Some(lo), Some(hi)) = (inp["hard_min"].as_f64(), inp["hard_max"].as_f64()) {
                            constraints.push(format!("hard: {lo}..{hi}"));
                        }
                        if let Some(unit) = inp["unit"].as_str() { constraints.push(format!("unit: {unit}")); }
                        if let Some(step) = inp["step"].as_f64() { constraints.push(format!("step: {step}")); }
                        if let Some(dp) = inp["display_decimal_places"].as_u64() { constraints.push(format!("decimals: {dp}")); }
                        if !constraints.is_empty() {
                            out.push_str(&format!("    Constraints: {}\n", constraints.join(", ")));
                        }

                        if let Some(impls) = inp["implementations"].as_array().filter(|a| !a.is_empty()) {
                            let types: Vec<String> = impls.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                            out.push_str(&format!("    Types: {}\n", types.join(", ")));
                        }
                    }
                }

                out.push_str(&format!("\n## Output\n`{}`\n", node["output_type"].as_str().unwrap_or("?")));

                if let Some(props) = node["properties"].as_str() {
                    out.push_str(&format!("\n**Properties panel:** `{props}`\n"));
                }
                if let Some(shader) = node["shader_node"].as_str() {
                    out.push_str(&format!("\n**Shader node:** `{shader}`\n"));
                }

                return Ok(vec![ToolContent::Text { text: out }]);
            }
        }
    }
    Err(anyhow::anyhow!("Node not found: `{node_id}`"))
}
