use anyhow::{Result, Context};
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

use crate::mcp_protocol::ToolContent;

// Embed the node catalog at compile time so the binary is self-contained.
// Path is relative to this source file: src/ -> graphite-mcp-server/ -> tools/ -> repo root
const CATALOG_JSON: &str = include_str!("../../../node_catalog.json");

static CATALOG: OnceLock<Value> = OnceLock::new();

fn get_catalog() -> Result<&'static Value> {
    if let Some(c) = CATALOG.get() {
        return Ok(c);
    }
    let catalog: Value = serde_json::from_str(CATALOG_JSON).context("Failed to parse embedded node catalog")?;
    let _ = CATALOG.set(catalog);
    Ok(CATALOG.get().unwrap())
}

// Editor reference for headed mode. We use a wrapper type to make it thread-safe.
struct EditorWrapper {
    ptr: *mut graphite_editor::application::Editor,
}

// SAFETY: The editor is only accessed from the main thread via the event loop.
// The MCP server thread sends commands via channels, and the main thread processes them.
unsafe impl Send for EditorWrapper {}
unsafe impl Sync for EditorWrapper {}

static EDITOR_WRAPPER: OnceLock<Mutex<Option<EditorWrapper>>> = OnceLock::new();

fn get_editor_wrapper() -> &'static Mutex<Option<EditorWrapper>> {
    EDITOR_WRAPPER.get_or_init(|| Mutex::new(None))
}

/// Set the editor reference for headed mode. Called by the desktop app's main thread.
///
/// # Safety
/// The caller must ensure the editor lives as long as the MCP server runs,
/// and that `clear_editor()` is called before the editor is dropped.
pub unsafe fn set_editor(editor: &mut graphite_editor::application::Editor) {
    let wrapper = get_editor_wrapper();
    *wrapper.lock().unwrap() = Some(EditorWrapper {
        ptr: editor as *mut _,
    });
}

/// Clear the editor reference. Called when the MCP server shuts down.
pub fn clear_editor() {
    if let Some(wrapper) = EDITOR_WRAPPER.get() {
        *wrapper.lock().unwrap() = None;
    }
}

fn with_editor<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut graphite_editor::application::Editor) -> R,
{
    let wrapper_lock = get_editor_wrapper();
    let guard = wrapper_lock.lock().unwrap();
    let wrapper = guard.as_ref().context("No editor connected (standalone mode)")?;
    // SAFETY: set_editor guarantees the editor outlives this reference
    Ok(f(unsafe { &mut *wrapper.ptr }))
}

pub async fn dispatch_tool_call(name: &str, args: Value) -> Result<Vec<ToolContent>> {
    match name {
        "get_node_catalog" => get_node_catalog(&args).await,
        "get_node_details" => get_node_details(&args).await,
        "list_documents" => list_documents(&args).await,
        "create_document" => create_document(&args).await,
        "get_layer_tree" => get_layer_tree(&args).await,
        "create_rectangle" => create_rectangle(&args).await,
        "create_ellipse" => create_ellipse(&args).await,
        "create_line" => create_line(&args).await,
        "create_text" => create_text_layer(&args).await,
        "select_layer" => select_layer(&args).await,
        "delete_selected" => delete_selected(&args).await,
        "get_selection" => get_selection(&args).await,
        "get_layer_properties" => get_layer_properties(&args).await,
        "set_fill_color" => set_fill_color(&args).await,
        "set_stroke" => set_stroke(&args).await,
        "set_opacity" => set_opacity(&args).await,
        "set_blend_mode" => set_blend_mode(&args).await,
        "move_layer" => move_layer(&args).await,
        "undo" => dispatch_undo_redo("undo").await,
        "redo" => dispatch_undo_redo("redo").await,
        "get_node_graph" => get_node_graph(&args).await,
        "activate_tool" => activate_tool(&args).await,
        "zoom_to_fit" => zoom_to_fit(&args).await,
        "set_viewport" => set_viewport(&args).await,
        "show_editor" => show_editor(&args).await,
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

// -- Node catalog tools (work directly from JSON catalog) --

async fn get_node_catalog(args: &Value) -> Result<Vec<ToolContent>> {
    let catalog = get_catalog()?;
    let category_filter = args.get("category").and_then(|v| v.as_str());
    let search_filter = args.get("search").and_then(|v| v.as_str());

    let mut output = String::new();
    let total = catalog["total_nodes"].as_u64().unwrap_or(0);
    let num_cats = catalog["categories"].as_object().map(|o| o.len()).unwrap_or(0);
    output.push_str(&format!("Graphite Node Catalog: {total} nodes in {num_cats} categories\n\n"));

    let categories = catalog["categories"].as_object().context("Invalid catalog")?;
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

async fn get_node_details(args: &Value) -> Result<Vec<ToolContent>> {
    let node_id = args.get("node_id").and_then(|v| v.as_str())
        .context("Missing required parameter: node_id")?;
    let catalog = get_catalog()?;
    let categories = catalog["categories"].as_object().context("Invalid catalog")?;

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

// -- Editor tools: dispatch via Editor::handle_message() when connected --

use graphite_editor::messages::prelude::*;

async fn send_editor_command(command: &str, args: Value) -> Result<Vec<ToolContent>> {
    match with_editor(|editor| {
        let message = match command {
            "list_documents" => Some(Message::Portfolio(PortfolioMessage::UpdateOpenDocumentsList)),
            "create_document" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
                Some(Message::Portfolio(PortfolioMessage::NewDocumentWithName { name }))
            }
            "delete_selected" => Some(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DeleteSelectedLayers))),
            "undo" => Some(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DocumentHistoryBackward))),
            "redo" => Some(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DocumentHistoryForward))),
            "zoom_to_fit" => Some(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::ZoomCanvasToFitAll))),
            _ => None,
        };

        if let Some(msg) = message {
            let responses = editor.handle_message(msg);
            let text = format_responses(&responses);
            Ok::<_, anyhow::Error>(text)
        } else {
            Ok(format!("[editor command: {command}] (handler not yet implemented)"))
        }
    }) {
        Ok(text) => Ok(vec![ToolContent::Text { text: text? }]),
        Err(e) => Ok(vec![ToolContent::Text { text: format!("Error: {e}") }]),
    }
}

fn format_responses(responses: &[FrontendMessage]) -> String {
    if responses.is_empty() {
        "OK".into()
    } else {
        let mut out = String::new();
        for msg in responses {
            // Extract useful info from frontend messages
            match msg {
                FrontendMessage::UpdateOpenDocumentsList { .. } => {
                    out.push_str("(document list updated)\n");
                }
                _ => {
                    out.push_str(&format!("({:?})\n", std::mem::discriminant(msg)));
                }
            }
        }
        out
    }
}

async fn list_documents(_args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("list_documents", Value::Object(serde_json::Map::new())).await
}

async fn create_document(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("create_document", args.clone()).await
}

async fn get_layer_tree(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("get_layer_tree", args.clone()).await
}

async fn create_rectangle(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("create_rectangle", args.clone()).await
}

async fn create_ellipse(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("create_ellipse", args.clone()).await
}

async fn create_line(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("create_line", args.clone()).await
}

async fn create_text_layer(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("create_text", args.clone()).await
}

async fn select_layer(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("select_layer", args.clone()).await
}

async fn delete_selected(_args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("delete_selected", Value::Object(serde_json::Map::new())).await
}

async fn get_selection(_args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("get_selection", Value::Object(serde_json::Map::new())).await
}

async fn get_layer_properties(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("get_layer_properties", args.clone()).await
}

async fn set_fill_color(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("set_fill_color", args.clone()).await
}

async fn set_stroke(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("set_stroke", args.clone()).await
}

async fn set_opacity(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("set_opacity", args.clone()).await
}

async fn set_blend_mode(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("set_blend_mode", args.clone()).await
}

async fn move_layer(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("move_layer", args.clone()).await
}

async fn dispatch_undo_redo(action: &str) -> Result<Vec<ToolContent>> {
    send_editor_command(action, Value::Object(serde_json::Map::new())).await
}

async fn get_node_graph(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("get_node_graph", args.clone()).await
}

async fn activate_tool(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("activate_tool", args.clone()).await
}

async fn zoom_to_fit(_args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("zoom_to_fit", Value::Object(serde_json::Map::new())).await
}

async fn set_viewport(args: &Value) -> Result<Vec<ToolContent>> {
    send_editor_command("set_viewport", args.clone()).await
}

/// Open the Graphite editor window so the user can see the current document.
async fn show_editor(_args: &Value) -> Result<Vec<ToolContent>> {
    // Try the .app bundle first (macOS), then fall back to the raw binary
    let home = std::env::var("HOME").unwrap_or_default();
    let app_bundle = format!("{}/tools/Graphite.app", home);

    let result = if std::path::Path::new(&app_bundle).exists() {
        std::process::Command::new("open").arg(&app_bundle).spawn()
    } else if let Some(bin) = which_graphite_binary() {
        std::process::Command::new(&bin).spawn()
    } else {
        return Err(anyhow::anyhow!("Could not find Graphite binary or app bundle"));
    };

    match result {
        Ok(_) => Ok(vec![ToolContent::Text {
            text: "Graphite editor window is opening. The user can now see and interact with the document.".to_string(),
        }]),
        Err(e) => Err(anyhow::anyhow!("Failed to launch Graphite: {e}")),
    }
}

fn which_graphite_binary() -> Option<String> {
    // Check ~/tools/bin/graphite first
    let home = std::env::var("HOME").unwrap_or_default();
    let tools_bin = format!("{}/tools/bin/graphite", home);
    if std::path::Path::new(&tools_bin).exists() {
        return Some(tools_bin);
    }

    // Check PATH via `which`
    if let Ok(output) = std::process::Command::new("which").arg("graphite").output() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }

    // Check current exe's directory (same dir as graphite-mcp)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("graphite");
            if sibling.exists() {
                return Some(sibling.to_string_lossy().to_string());
            }
        }
    }

    None
}
