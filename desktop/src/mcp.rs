//! MCP server integration for the desktop app.
//!
//! When `--mcp` is passed, this module starts a thread that reads JSON-RPC
//! messages from stdin and dispatches tool calls to the main thread via
//! `AppEvent::McpToolCall`.

use crate::event::{AppEvent, AppEventScheduler};
use std::io::{self, BufRead, BufReader, Write};
use std::sync::mpsc;
use std::thread;

pub(crate) struct McpHandle {
	thread: Option<thread::JoinHandle<()>>,
	shutdown_sender: mpsc::Sender<()>,
}

impl Drop for McpHandle {
	fn drop(&mut self) {
		let _ = self.shutdown_sender.send(());
		let _ = self.thread.take().expect("McpHandle can only be dropped once").join();
	}
}

pub(crate) fn start(app_event_scheduler: AppEventScheduler) -> McpHandle {
	let (shutdown_sender, shutdown_receiver) = mpsc::channel();

	let thread = thread::Builder::new()
		.name("mcp-server".to_string())
		.spawn(move || run(app_event_scheduler, shutdown_receiver))
		.expect("Failed to spawn MCP server thread");

	McpHandle {
		shutdown_sender,
		thread: Some(thread),
	}
}

fn run(app_event_scheduler: AppEventScheduler, shutdown_receiver: mpsc::Receiver<()>) {
	tracing::info!("MCP server thread started");

	let stdin = io::stdin();
	let mut reader = BufReader::new(stdin.lock());
	let mut stdout = io::stdout();

	let mut buf = String::new();
	loop {
		buf.clear();

		// Check for shutdown
		if shutdown_receiver.try_recv().is_ok() {
			break;
		}

		// Read a line from stdin (non-blocking check first)
		match reader.read_line(&mut buf) {
			Ok(0) => break, // EOF
			Ok(_) => {}
			Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {
				thread::sleep(std::time::Duration::from_millis(10));
				continue;
			}
			Err(e) => {
				tracing::error!("MCP server read error: {}", e);
				break;
			}
		}

		let trimmed = buf.trim();
		if trimmed.is_empty() {
			continue;
		}

		// Parse JSON-RPC request
		let request: serde_json::Value = match serde_json::from_str(trimmed) {
			Ok(v) => v,
			Err(e) => {
				tracing::warn!("MCP server: invalid JSON: {}", e);
				continue;
			}
		};

		let response = handle_jsonrpc(&request, &app_event_scheduler);

		// Write response
		let resp_str = match serde_json::to_string(&response) {
			Ok(s) => s,
			Err(e) => {
				tracing::error!("MCP server: failed to serialize response: {}", e);
				continue;
			}
		};

		if let Err(e) = writeln!(stdout, "{}", resp_str) {
			tracing::error!("MCP server: write error: {}", e);
			break;
		}
		if let Err(e) = stdout.flush() {
			tracing::error!("MCP server: flush error: {}", e);
			break;
		}
	}

	tracing::info!("MCP server thread stopped");
}

fn handle_jsonrpc(request: &serde_json::Value, app_event_scheduler: &AppEventScheduler) -> serde_json::Value {
	let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
	let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
	let params = request.get("params").cloned().unwrap_or(serde_json::json!({}));

	match method {
		"initialize" => serde_json::json!({
			"jsonrpc": "2.0",
			"id": id,
			"result": {
				"protocolVersion": "2024-11-05",
				"capabilities": {
					"tools": { "listChanged": false }
				},
				"serverInfo": {
					"name": "graphite-mcp-server",
					"version": "0.1.0"
				}
			}
		}),
		"notifications/initialized" => serde_json::Value::Null,
		"tools/list" => {
			let tools = graphite_mcp_server::tools::list_tools();
			serde_json::json!({
				"jsonrpc": "2.0",
				"id": id,
				"result": { "tools": tools }
			})
		}
		"tools/call" => {
			let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
			let arguments = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));

			// Dispatch to editor via AppEvent
			let (response_sender, response_receiver) = mpsc::channel();
			app_event_scheduler.schedule(AppEvent::McpToolCall {
				tool_name: tool_name.to_string(),
				args: arguments,
				response_sender,
			});

			// Wait for response from the main thread
			match response_receiver.recv_timeout(std::time::Duration::from_secs(30)) {
				Ok(Ok(content)) => {
					let tool_content: Vec<serde_json::Value> = content
						.into_iter()
						.map(|v| serde_json::json!({ "type": "text", "text": v }))
						.collect();
					serde_json::json!({
						"jsonrpc": "2.0",
						"id": id,
						"result": {
							"content": tool_content
						}
					})
				}
				Ok(Err(e)) => serde_json::json!({
					"jsonrpc": "2.0",
					"id": id,
					"result": {
						"content": [{ "type": "text", "text": format!("Error: {}", e) }],
						"isError": true
					}
				}),
				Err(_) => serde_json::json!({
					"jsonrpc": "2.0",
					"id": id,
					"result": {
						"content": [{ "type": "text", "text": "Error: Tool call timed out" }],
						"isError": true
					}
				}),
			}
		}
		"resources/list" => serde_json::json!({
			"jsonrpc": "2.0",
			"id": id,
			"result": { "resources": [] }
		}),
		_ => serde_json::json!({
			"jsonrpc": "2.0",
			"id": id,
			"error": {
				"code": -32601,
				"message": format!("Method not found: {}", method)
			}
		}),
	}
}
