use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::tools;

// -- JSON-RPC 2.0 types --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
}

// -- MCP types --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, #[serde(rename = "mimeType")] mime_type: String },
}

// -- Server --

pub struct McpServer {
    initialized: bool,
}

impl McpServer {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    pub async fn handle(&mut self, msg: JsonRpcMessage) -> Option<JsonRpcResponse> {
        match msg {
            JsonRpcMessage::Request(req) => self.handle_request(req).await,
        }
    }

    async fn handle_request(&mut self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        match req.method.as_str() {
            "initialize" => {
                self.initialized = true;
                Some(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: Some(serde_json::json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": { "listChanged": false },
                            "resources": { "listChanged": false }
                        },
                        "serverInfo": {
                            "name": "graphite-mcp-server",
                            "version": "0.1.0"
                        }
                    })),
                    error: None,
                })
            }
            "notifications/initialized" => None,
            "tools/list" => {
                let tool_list = tools::list_tools();
                Some(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: Some(serde_json::json!({ "tools": tool_list })),
                    error: None,
                })
            }
			"tools/call" => {
                let tool_name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = req.params.get("arguments").cloned().unwrap_or(Value::Object(serde_json::Map::new()));

                match tools::call_tool_async(tool_name, arguments).await {
                    Ok(content) => Some(JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: req.id,
                        result: Some(serde_json::to_value(ToolCallResult {
                            content,
                            is_error: None,
                        }).unwrap()),
                        error: None,
                    }),
                    Err(e) => Some(JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: req.id,
                        result: Some(serde_json::to_value(ToolCallResult {
                            content: vec![ToolContent::Text { text: format!("Error: {e}") }],
                            is_error: Some(true),
                        }).unwrap()),
                        error: None,
                    }),
                }
            }
            "resources/list" => Some(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: Some(serde_json::json!({ "resources": [] })),
                error: None,
            }),
            _ => Some(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                }),
            }),
        }
    }
}
