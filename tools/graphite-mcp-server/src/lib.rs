pub mod editor_bridge;
pub mod mcp_protocol;
pub mod tools;

use mcp_protocol::{JsonRpcMessage, McpServer};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Run the MCP server on stdin/stdout, dispatching tool calls to the provided editor.
///
/// This is the "headed" mode entry point — the editor is already running and we
/// borrow a mutable reference to it for the duration of the session.
pub async fn run_mcp_server(editor: &mut graphite_editor::application::Editor) -> anyhow::Result<()> {
    // SAFETY: The editor is owned by the caller and must outlive this MCP server session.
    // The caller must call clear_editor() before dropping the editor.
    unsafe { editor_bridge::set_editor(editor); }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut writer = stdout;

    let mut server = McpServer::new();

    eprintln!("graphite-mcp-server started, waiting for JSON-RPC on stdin...");

    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            break;
        }

        let msg = match serde_json::from_str::<JsonRpcMessage>(buf.trim()) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Failed to parse JSON-RPC: {e}");
                continue;
            }
        };

        let response = server.handle(msg).await;
        if let Some(resp) = response {
            let mut resp_str = serde_json::to_string(&resp)?;
            resp_str.push('\n');
            writer.write_all(resp_str.as_bytes()).await?;
            writer.flush().await?;
        }
    }

    editor_bridge::clear_editor();
    Ok(())
}

/// Run the MCP server in standalone mode (no editor, catalog-only tools).
/// This is the headless/standalone binary entry point.
pub async fn run_standalone() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut writer = stdout;

    let mut server = McpServer::new();

    eprintln!("graphite-mcp-server (standalone) started, waiting for JSON-RPC on stdin...");

    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            break;
        }

        let msg = match serde_json::from_str::<JsonRpcMessage>(buf.trim()) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Failed to parse JSON-RPC: {e}");
                continue;
            }
        };

        let response = server.handle(msg).await;
        if let Some(resp) = response {
            let mut resp_str = serde_json::to_string(&resp)?;
            resp_str.push('\n');
            writer.write_all(resp_str.as_bytes()).await?;
            writer.flush().await?;
        }
    }

    Ok(())
}
