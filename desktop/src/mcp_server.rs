//! WebSocket MCP server for the Graphite desktop app.
//!
//! When started with `--mcp-server`, this module runs a WebSocket server on localhost
//! that accepts MCP tool calls from external clients (like the lightweight MCP proxy).

use crate::event::{AppEvent, AppEventScheduler};
use futures_util::stream::StreamExt;
use futures_util::sink::SinkExt;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;

pub(crate) struct McpServerHandle {
    shutdown_sender: mpsc::Sender<()>,
}

impl Drop for McpServerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_sender.blocking_send(());
    }
}

/// Start the WebSocket MCP server on the specified port.
pub(crate) fn start(port: u16, app_event_scheduler: AppEventScheduler) -> McpServerHandle {
    let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime for MCP server");
        runtime.block_on(async move {
            if let Err(e) = run_server(port, app_event_scheduler, shutdown_receiver).await {
                tracing::error!("MCP WebSocket server error: {e}");
            }
        });
    });

    McpServerHandle { shutdown_sender }
}

async fn run_server(
    port: u16,
    app_event_scheduler: AppEventScheduler,
    mut shutdown_receiver: mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("MCP WebSocket server listening on ws://localhost:{port}");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, peer)) => {
                        tracing::info!("MCP client connected from {peer}");
                        let scheduler = app_event_scheduler.clone();
                        tokio::spawn(handle_connection(stream, scheduler));
                    }
                    Err(e) => {
                        tracing::error!("Failed to accept connection: {e}");
                    }
                }
            }
            _ = shutdown_receiver.recv() => {
                tracing::info!("MCP WebSocket server shutting down");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    app_event_scheduler: AppEventScheduler,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::error!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut write, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                tracing::error!("WebSocket read error: {e}");
                break;
            }
        };

        if msg.is_text() {
            let text = msg.to_text().unwrap_or("");
            let response = handle_jsonrpc(text, &app_event_scheduler).await;

            if let Some(resp) = response {
                if let Err(e) = write.send(tokio_tungstenite::tungstenite::Message::Text(resp.into())).await {
                    tracing::error!("WebSocket write error: {e}");
                    break;
                }
            }
        } else if msg.is_close() {
            break;
        }
    }

    tracing::info!("MCP client disconnected");
}

async fn handle_jsonrpc(
    text: &str,
    app_event_scheduler: &AppEventScheduler,
) -> Option<String> {
    let request: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Invalid JSON: {e}");
            return Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("Parse error: {e}")}
            }).to_string());
        }
    };

    let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(serde_json::json!({}));

    match method {
        "initialize" => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "graphite-mcp", "version": "0.1.0"}
            }
        }).to_string()),

        "notifications/initialized" => None,

        "tools/list" => {
            let tools = graphite_mcp_server::tools::list_tools();
            Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": tools}
            }).to_string())
        }

        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));

            // Dispatch to editor via AppEvent
            let (response_sender, response_receiver) = std::sync::mpsc::channel();
            app_event_scheduler.schedule(AppEvent::McpToolCall {
                tool_name: tool_name.to_string(),
                args: arguments,
                response_sender,
            });

            // Wait for response from the main thread (blocking in async context is okay here
            // because we're in a dedicated WebSocket handler task)
            let result = tokio::task::spawn_blocking(move || {
                response_receiver.recv_timeout(std::time::Duration::from_secs(30))
            }).await.unwrap_or(Err(std::sync::mpsc::RecvTimeoutError::Disconnected));

            match result {
                Ok(Ok(content)) => {
                    let tool_content: Vec<serde_json::Value> = content
                        .into_iter()
                        .map(|text| serde_json::json!({"type": "text", "text": text}))
                        .collect();
                    Some(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"content": tool_content}
                    }).to_string())
                }
                Ok(Err(e)) => Some(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": format!("Error: {e}")}],
                        "isError": true
                    }
                }).to_string()),
                Err(_) => Some(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": "Error: Tool call timed out"}],
                        "isError": true
                    }
                }).to_string()),
            }
        }

        "resources/list" => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"resources": []}
        }).to_string()),

        _ => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("Method not found: {method}")}
        }).to_string()),
    }
}
