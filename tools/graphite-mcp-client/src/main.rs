//! MCP relay server for Graphite.
//!
//! This acts as a bridge between the AI agent (stdio JSON-RPC) and the
//! Graphite web app (WebSocket). The browser connects to this server,
//! and the agent communicates via stdin/stdout.
//!
//! Flow:
//!   Agent → stdin → relay → WebSocket → Browser (WASM editor)
//!   Browser → WebSocket → relay → stdout → Agent

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type WsWriter = Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<tokio::net::TcpStream>, Message>>>;

#[tokio::main]
async fn main() -> Result<()> {
	let port: u16 = std::env::var("GRAPHITE_MCP_PORT")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(8081);

	// Shared state: the currently connected browser WebSocket
	let browser_ws: Arc<Mutex<Option<WsWriter>>> = Arc::new(Mutex::new(None));

	// Channel: browser sends responses back to stdin reader task
	let (response_tx, mut response_rx) = mpsc::channel::<String>(64);

	// Start WebSocket server (browser connects here)
	let ws_addr = format!("127.0.0.1:{port}");
	let listener = TcpListener::bind(&ws_addr).await?;
	eprintln!("[graphite-mcp-client] WebSocket server listening on ws://localhost:{port}");
	eprintln!("[graphite-mcp-client] Waiting for browser to connect...");

	let browser_ws_clone = browser_ws.clone();
	let response_tx_clone = response_tx.clone();
	tokio::spawn(async move {
		loop {
			match listener.accept().await {
				Ok((stream, _)) => {
					eprintln!("[graphite-mcp-client] Browser connected");
					let ws_stream = match accept_async(stream).await {
						Ok(ws) => ws,
						Err(e) => {
							eprintln!("[graphite-mcp-client] WebSocket handshake failed: {e}");
							continue;
						}
					};

					// Split into read/write
					let (write, read) = ws_stream.split();
					let write = Arc::new(Mutex::new(write));

					// Store the write half so the stdin task can send requests to the browser
					{
						let mut guard = browser_ws_clone.lock().await;
						*guard = Some(write.clone());
					}

					// Read responses from browser and forward to stdout
					let tx = response_tx_clone.clone();
					let ws_for_cleanup = browser_ws_clone.clone();
					tokio::spawn(async move {
						let mut read = read;
						while let Some(msg) = read.next().await {
							match msg {
								Ok(Message::Text(text)) => {
									let _ = tx.send(text.to_string()).await;
								}
								Ok(Message::Close(_)) => {
									eprintln!("[graphite-mcp-client] Browser disconnected");
									break;
								}
								Err(e) => {
									eprintln!("[graphite-mcp-client] WebSocket read error: {e}");
									break;
								}
								_ => {}
							}
						}

						// Clear the browser connection
						let mut guard = ws_for_cleanup.lock().await;
						*guard = None;
						eprintln!("[graphite-mcp-client] Browser connection cleared, waiting for reconnect...");
					});
				}
				Err(e) => {
					eprintln!("[graphite-mcp-client] Accept error: {e}");
				}
			}
		}
	});

	// Read JSON-RPC from stdin (from the agent) and forward to browser
	let stdin = io::stdin();
	let mut reader = BufReader::new(stdin);
	let mut stdout = io::stdout();
	let mut buf = String::new();

	loop {
		// Check if we have a browser connected
		{
			let guard = browser_ws.lock().await;
			if guard.is_none() {
				// Wait for browser to connect, but keep polling
			}
		}

		buf.clear();
		let n = reader.read_line(&mut buf).await?;
		if n == 0 {
			// stdin EOF — the agent hasn't connected yet. Wait a bit and retry.
			// This allows the relay to stay alive even when launched without stdin.
			tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
			continue;
		}

		let line = buf.trim().to_string();
		if line.is_empty() {
			continue;
		}

		// Wait for the browser to connect before forwarding (up to 10 seconds)
		let mut waited = 0u64;
		loop {
			let guard = browser_ws.lock().await;
			if guard.is_some() {
				drop(guard);
				break;
			}
			drop(guard);
			if waited >= 10_000 {
				break;
			}
			tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
			waited += 200;
		}

		// Forward to browser WebSocket
		let sent = {
			let guard = browser_ws.lock().await;
			if let Some(write) = guard.as_ref() {
				let mut w = write.lock().await;
				w.send(Message::Text(line.clone().into())).await.is_ok()
			} else {
				false
			}
		};

		if !sent {
			// No browser connected — return an error response
			let id = extract_id(&line);
			let error_response = serde_json::json!({
				"jsonrpc": "2.0",
				"id": id,
				"error": {
					"code": -32000,
					"message": "Graphite web app is not connected. Make sure to open http://localhost:8080 in your browser and wait for the MCP bridge to connect."
				}
			});
			let mut resp = serde_json::to_string(&error_response)?;
			resp.push('\n');
			stdout.write_all(resp.as_bytes()).await?;
			stdout.flush().await?;
			continue;
		}

		// Skip waiting for a response if this is a notification (no "id" field)
		let is_notification = serde_json::from_str::<serde_json::Value>(&line)
			.ok()
			.and_then(|v| v.get("id").cloned())
			.is_none();

		if is_notification {
			continue;
		}

		// Wait for the response from the browser (with a 15-second timeout)
		match tokio::time::timeout(tokio::time::Duration::from_secs(15), response_rx.recv()).await {
			Ok(Some(resp_text)) => {
				let mut resp = resp_text;
				resp.push('\n');
				stdout.write_all(resp.as_bytes()).await?;
				stdout.flush().await?;
			}
			Ok(None) => {
				// Channel closed (browser disconnected)
				let id = extract_id(&line);
				let error_response = serde_json::json!({
					"jsonrpc": "2.0",
					"id": id,
					"error": {
						"code": -32000,
						"message": "Browser disconnected during tool call"
					}
				});
				let mut resp = serde_json::to_string(&error_response)?;
				resp.push('\n');
				stdout.write_all(resp.as_bytes()).await?;
				stdout.flush().await?;
			}
			Err(_) => {
				// Timeout
				let id = extract_id(&line);
				let error_response = serde_json::json!({
					"jsonrpc": "2.0",
					"id": id,
					"error": {
						"code": -32001,
						"message": "Tool call timed out (browser did not respond within 15 seconds)"
					}
				});
				let mut resp = serde_json::to_string(&error_response)?;
				resp.push('\n');
				stdout.write_all(resp.as_bytes()).await?;
				stdout.flush().await?;
			}
		}
	}

	Ok(())
}

/// Extract the `id` field from a JSON-RPC request line (best-effort).
fn extract_id(line: &str) -> serde_json::Value {
	if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
		v.get("id").cloned().unwrap_or(serde_json::Value::Null)
	} else {
		serde_json::Value::Null
	}
}