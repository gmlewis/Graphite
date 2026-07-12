// Browser-side MCP bridge: connects to the MCP relay server via WebSocket
// and dispatches incoming JSON-RPC tool calls to the WASM editor.

import type { EditorWrapper } from "/wrapper/pkg/graphite_wasm_wrapper";

const MCP_RELAY_URL = "ws://localhost:8081";
const RECONNECT_INTERVAL = 2000;

// Tool list — must match the tools handled in the WASM wrapper's mcpToolCall
const TOOLS = [
	{ name: "get_node_catalog", description: "List all available nodes with their categories, inputs, outputs, and parameters.", inputSchema: { type: "object", properties: { category: { type: "string" }, search: { type: "string" } } } },
	{ name: "get_node_details", description: "Get detailed information about a specific node by its ID.", inputSchema: { type: "object", properties: { node_id: { type: "string" } }, required: ["node_id"] } },
	{ name: "list_documents", description: "List all open documents.", inputSchema: { type: "object", properties: {} } },
	{ name: "create_document", description: "Create a new blank document.", inputSchema: { type: "object", properties: { name: { type: "string" } } } },
	{ name: "get_layer_tree", description: "Get the full layer tree with IDs, names, visibility, and lock state.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_selection", description: "List selected layer IDs and names.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_layer_properties", description: "Get properties of a specific layer by ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string" } }, required: ["layer_id"] } },
	{ name: "get_node_graph", description: "Get the node graph for a specific layer.", inputSchema: { type: "object", properties: { layer_id: { type: "string" } }, required: ["layer_id"] } },
	{ name: "create_rectangle", description: "Create a rectangle shape with position, size, fill, and corner radius.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, width: { type: "number" }, height: { type: "number" }, fill_color: { type: "string" }, corner_radius: { type: "number" } } } },
	{ name: "create_ellipse", description: "Create an ellipse shape with center, radii, and fill.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, radius_x: { type: "number" }, radius_y: { type: "number" }, fill_color: { type: "string" } } } },
	{ name: "create_line", description: "Create a line with endpoints, stroke color/width.", inputSchema: { type: "object", properties: { x1: { type: "number" }, y1: { type: "number" }, x2: { type: "number" }, y2: { type: "number" }, stroke_color: { type: "string" }, stroke_width: { type: "number" } } } },
	{ name: "create_text", description: "Create a text layer with position, content, font size, and color.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, text: { type: "string" }, font_size: { type: "number" }, fill_color: { type: "string" } } } },
	{ name: "select_layer", description: "Select a layer by its ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string" } }, required: ["layer_id"] } },
	{ name: "delete_selected", description: "Delete all selected layers.", inputSchema: { type: "object", properties: {} } },
	{ name: "undo", description: "Undo the last action.", inputSchema: { type: "object", properties: {} } },
	{ name: "redo", description: "Redo the last undone action.", inputSchema: { type: "object", properties: {} } },
	{ name: "set_fill_color", description: "Set fill opacity for selected layers.", inputSchema: { type: "object", properties: { opacity: { type: "number" } } } },
	{ name: "set_opacity", description: "Set opacity for selected layers.", inputSchema: { type: "object", properties: { opacity: { type: "number" } } } },
	{ name: "set_blend_mode", description: "Set blend mode for selected layers.", inputSchema: { type: "object", properties: { blend_mode: { type: "string" } } } },
	{ name: "set_stroke", description: "Set stroke color and width on the first selected layer.", inputSchema: { type: "object", properties: { color: { type: "string" }, width: { type: "number" } } } },
	{ name: "move_layer", description: "Move selected layers by dx, dy.", inputSchema: { type: "object", properties: { dx: { type: "number" }, dy: { type: "number" } } } },
	{ name: "activate_tool", description: "Activate a tool (Select, Pen, Path, Line, Rectangle, Ellipse, Freehand, Text, Fill, Gradient, Eyedropper).", inputSchema: { type: "object", properties: { tool: { type: "string" } } } },
	{ name: "zoom_to_fit", description: "Zoom canvas to fit all layers.", inputSchema: { type: "object", properties: {} } },
	{ name: "set_viewport", description: "Set viewport zoom.", inputSchema: { type: "object", properties: { zoom: { type: "number" } } } },
	{ name: "get_document_info", description: "Get document metadata: name, layer count, artboard count, selection state.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_layer_bounds", description: "Get the bounding box of a specific layer by ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string" } }, required: ["layer_id"] } },
	{ name: "get_selected_layer_info", description: "Get detailed info about all selected layers including bounds, visibility, and lock state.", inputSchema: { type: "object", properties: {} } },
	{ name: "import_svg", description: "Import a complete SVG string as a new layer group. Accepts any valid SVG markup including <rect>, <circle>, <ellipse>, <path>, <line>, <polygon>, <polyline>, <g>, <text>, <defs>, <linearGradient>, <radialGradient>, etc. The SVG is placed at document origin (0,0) using absolute coordinates from the SVG.", inputSchema: { type: "object", properties: { svg: { type: "string", description: "Complete SVG string with root <svg> element" }, name: { type: "string", description: "Optional name for the imported layer group" } }, required: ["svg"] } },
	{ name: "create_path", description: "Create a vector path from SVG path data (d attribute). The path is filled and/or stroked.", inputSchema: { type: "object", properties: { d: { type: "string", description: "SVG path data string, e.g. 'M 0 0 L 100 0 L 100 100 Z'" }, fill_color: { type: "string", description: "Fill color as hex, e.g. '#FF0000'" }, stroke_color: { type: "string", description: "Stroke color as hex" }, stroke_width: { type: "number", description: "Stroke width in pixels" }, x: { type: "number", description: "X translation" }, y: { type: "number", description: "Y translation" } }, required: ["d"] } },
	{ name: "batch_create", description: "Create multiple shapes in a single call. Each shape object supports: type (rectangle|ellipse|line|text), x, y, width, height, radius_x, radius_y, x1, y1, x2, y2, fill_color, stroke_color, stroke_width, text, font_size, corner_radius. All coordinates are in document space.", inputSchema: { type: "object", properties: { shapes: { type: "array", items: { type: "object" }, description: "Array of shape objects to create" } }, required: ["shapes"] } },
];

export function startMcpBridge(editor: EditorWrapper): void {
	let ws: WebSocket | undefined;
	let connected = false;

	function connect() {
		try {
			ws = new WebSocket(MCP_RELAY_URL);
		} catch {
			scheduleReconnect();
			return;
		}

		ws.onopen = () => {
			connected = true;
			console.log("[MCP Bridge] Connected to relay at", MCP_RELAY_URL);
		};

		ws.onclose = () => {
			connected = false;
			console.log("[MCP Bridge] Disconnected from relay");
			scheduleReconnect();
		};

		ws.onerror = () => {
			if (connected) {
				connected = false;
				scheduleReconnect();
			}
		};

		ws.onmessage = (event) => {
			handleMessage(event.data as string);
		};
	}

	function scheduleReconnect() {
		setTimeout(() => {
			if (!connected) connect();
		}, RECONNECT_INTERVAL);
	}

	function sendMessage(text: string) {
		if (ws && ws.readyState === WebSocket.OPEN) {
			ws.send(text);
		} else {
			console.warn("[MCP Bridge] Cannot send response — WebSocket not open");
		}
	}

	function handleMessage(text: string) {
		let request: { jsonrpc: string; id: unknown; method: string; params?: Record<string, unknown> };
		try {
			request = JSON.parse(text);
		} catch {
			console.warn("[MCP Bridge] Failed to parse message:", text);
			return;
		}

		const { jsonrpc, id, method, params } = request;
		console.log("[MCP Bridge] Received:", method, "id:", id);

		if (method === "initialize") {
			sendMessage(JSON.stringify({
				jsonrpc, id,
				result: {
					protocolVersion: "2024-11-05",
					capabilities: { tools: { listChanged: false } },
					serverInfo: { name: "graphite-mcp", version: "0.1.0" },
				},
			}));
			return;
		}

		if (method === "notifications/initialized") {
			return;
		}

		if (method === "tools/list") {
			sendMessage(JSON.stringify({
				jsonrpc, id,
				result: { tools: TOOLS },
			}));
			return;
		}

		if (method === "tools/call") {
			const toolName = (params as { name?: string })?.name ?? "";
			const arguments_ = (params as { arguments?: Record<string, unknown> })?.arguments ?? {};
			console.log("[MCP Bridge] Tool call:", toolName);
			let resultJson: string;
			try {
				resultJson = editor.mcpToolCall(toolName, JSON.stringify(arguments_));
				console.log("[MCP Bridge] Tool result:", resultJson.slice(0, 200));
			} catch (e) {
				console.error("[MCP Bridge] Tool error:", e);
				resultJson = JSON.stringify({ content: [{ type: "text", text: `Error: ${e}` }], isError: true });
			}
			sendMessage(JSON.stringify({
				jsonrpc, id,
				result: JSON.parse(resultJson),
			}));
			console.log("[MCP Bridge] Response sent for", toolName);
			return;
		}

		if (method === "resources/list") {
			sendMessage(JSON.stringify({ jsonrpc, id, result: { resources: [] } }));
			return;
		}

		sendMessage(JSON.stringify({
			jsonrpc, id,
			error: { code: -32601, message: `Method not found: ${method}` },
		}));
	}

	connect();
}