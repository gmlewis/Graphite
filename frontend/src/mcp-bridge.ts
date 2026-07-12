// Browser-side MCP bridge: connects to the MCP relay server via WebSocket
// and dispatches incoming JSON-RPC tool calls to the WASM editor.

import type { EditorWrapper } from "/wrapper/pkg/graphite_wasm_wrapper";

const MCP_RELAY_URL = "ws://localhost:8081";
const RECONNECT_INTERVAL = 2000;

// Tool list — must match the tools handled in the WASM wrapper's mcpToolCall
const TOOLS = [
	{ name: "get_node_catalog", description: "Node catalog query. NOTE: through the browser relay this returns a pointer message only — use the standalone graphite-mcp-server binary for real catalog queries. Not useful for drawing.", inputSchema: { type: "object", properties: { category: { type: "string" }, search: { type: "string" } } } },
	{ name: "get_node_details", description: "Node detail query. NOTE: through the browser relay this returns a pointer message only — use the standalone graphite-mcp-server binary. Not useful for drawing.", inputSchema: { type: "object", properties: { node_id: { type: "string" } }, required: ["node_id"] } },
	{ name: "list_documents", description: "Refreshes the document list shown in the editor UI. Does NOT return a list of documents — the response is just 'Document list updated in UI'. To inspect the active document, use get_document_info instead.", inputSchema: { type: "object", properties: {} } },
	{ name: "create_document", description: "Create a new blank document and make it the active document. Use get_document_info afterwards to confirm.", inputSchema: { type: "object", properties: { name: { type: "string" } } } },
	{ name: "get_layer_tree", description: "Get a flat markdown list of all layers in the active document with their integer IDs, kind (layer/group/artboard), name, visibility and lock state. Use this to discover layer IDs for select_layer, get_layer_properties, etc.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_selection", description: "List selected layer IDs (integers) and names in the active document.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_layer_properties", description: "Get name, kind, visibility and lock state of a specific layer by its integer ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID as string or number, e.g. '42'" } }, required: ["layer_id"] } },
	{ name: "get_node_graph", description: "Dump the document node (implementation + inputs) backing a layer by its integer ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID" } }, required: ["layer_id"] } },
	{ name: "create_rectangle", description: "Create a rectangle shape. x,y is the top-left corner. fill_color is a hex string like '#FF0000'. corner_radius rounds corners (0 = sharp).", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, width: { type: "number" }, height: { type: "number" }, fill_color: { type: "string" }, corner_radius: { type: "number" } }, required: ["x", "y", "width", "height"] } },
	{ name: "create_ellipse", description: "Create an ellipse. x,y is the CENTER, not top-left. radius_x/radius_y are the half-widths. fill_color is hex like '#FF0000'.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, radius_x: { type: "number" }, radius_y: { type: "number" }, fill_color: { type: "string" } }, required: ["x", "y", "radius_x", "radius_y"] } },
	{ name: "create_line", description: "Create a straight line from (x1,y1) to (x2,y2) with the given stroke color (hex) and width.", inputSchema: { type: "object", properties: { x1: { type: "number" }, y1: { type: "number" }, x2: { type: "number" }, y2: { type: "number" }, stroke_color: { type: "string" }, stroke_width: { type: "number" } }, required: ["x1", "y1", "x2", "y2"] } },
	{ name: "create_text", description: "Create a text layer at x,y with the given content, font size and hex fill color. Uses the default Graphite font.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, text: { type: "string" }, font_size: { type: "number" }, fill_color: { type: "string" } }, required: ["x", "y", "text"] } },
	{ name: "select_layer", description: "Select a single layer by its integer ID. Replaces the current selection.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID as string or number" } }, required: ["layer_id"] } },
	{ name: "delete_selected", description: "Delete all currently selected layers.", inputSchema: { type: "object", properties: {} } },
	{ name: "undo", description: "Undo the last editor action.", inputSchema: { type: "object", properties: {} } },
	{ name: "redo", description: "Redo the last undone editor action.", inputSchema: { type: "object", properties: {} } },
	{ name: "set_fill_color", description: "Despite the name, this sets the fill OPACITY of the selected layers (0-100 percent), NOT the fill color. There is currently no tool to set the fill color of an existing layer via this interface — to make colored shapes, use create_rectangle/ellipse/path or import_svg with a fill_color argument when creating the shape.", inputSchema: { type: "object", properties: { opacity: { type: "number", description: "Fill opacity percentage 0-100" } }, required: ["opacity"] } },
	{ name: "set_opacity", description: "Set the layer opacity of the selected layers (0-100 percent).", inputSchema: { type: "object", properties: { opacity: { type: "number", description: "Opacity percentage 0-100" } }, required: ["opacity"] } },
	{ name: "set_blend_mode", description: "Set the blend mode of the selected layers. NOTE: the current implementation always dispatches Normal regardless of the argument — see editor_wrapper.rs.", inputSchema: { type: "object", properties: { blend_mode: { type: "string", description: "Blend mode name: Normal, Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion, Hue, Saturation, Color, Luminosity, etc." } }, required: ["blend_mode"] } },
	{ name: "set_stroke", description: "Set stroke color (hex) and width (px) on the first selected layer. Requires a selection.", inputSchema: { type: "object", properties: { color: { type: "string" }, width: { type: "number" } } } },
	{ name: "move_layer", description: "Nudge all selected layers by (dx, dy) document pixels.", inputSchema: { type: "object", properties: { dx: { type: "number" }, dy: { type: "number" } }, required: ["dx", "dy"] } },
	{ name: "activate_tool", description: "Activate an editor tool by name. One of: Select, Pen, Path, Line, Rectangle, Ellipse, Freehand, Text, Fill, Gradient, Eyedropper.", inputSchema: { type: "object", properties: { tool: { type: "string" } }, required: ["tool"] } },
	{ name: "zoom_to_fit", description: "Zoom the canvas to fit all layers. Call this after creating content so the user can see it.", inputSchema: { type: "object", properties: {} } },
	{ name: "set_viewport", description: "Set the viewport zoom factor (1.0 = 100%). Only zoom is supported through this tool — there is no pan argument.", inputSchema: { type: "object", properties: { zoom: { type: "number" } }, required: ["zoom"] } },
	{ name: "get_document_info", description: "Get metadata about the active document: name, layer count, artboard count, selected count. USE THIS FIRST to orient — do not use list_documents, which only refreshes the UI.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_layer_bounds", description: "Get the axis-aligned bounding box (min/max XY and size) of a layer by its integer ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID" } }, required: ["layer_id"] } },
	{ name: "get_selected_layer_info", description: "Detailed info about every selected layer: ID, kind, name, visibility, lock, bounding box. Richer than get_selection.", inputSchema: { type: "object", properties: {} } },
	{ name: "import_svg", description: "Import a complete SVG string as a new layer group. THIS IS THE MOST POWERFUL TOOL for complex artwork — construct one SVG with many elements (<rect>, <circle>, <ellipse>, <path>, <line>, <polygon>, <polyline>, <g>, <text>, <defs>, <linearGradient>, <radialGradient>, etc.) and the editor parses it in one shot. Far faster and more reliable than hundreds of individual create_* calls. The SVG is placed at document origin (0,0) using absolute coordinates from the SVG.", inputSchema: { type: "object", properties: { svg: { type: "string", description: "Complete SVG string with root <svg> element" }, name: { type: "string", description: "Optional name for the imported layer group" } }, required: ["svg"] } },
	{ name: "create_path", description: "Create a vector path from SVG path data (the 'd' attribute). Optionally fill (hex) and/or stroke (hex + width). x,y translate the path. Useful for freeform curves and polygons that don't fit the rectangle/ellipse/line primitives.", inputSchema: { type: "object", properties: { d: { type: "string", description: "SVG path data string, e.g. 'M 0 0 L 100 0 L 100 100 Z'" }, fill_color: { type: "string", description: "Fill color as hex, e.g. '#FF0000'" }, stroke_color: { type: "string", description: "Stroke color as hex" }, stroke_width: { type: "number", description: "Stroke width in pixels" }, x: { type: "number", description: "X translation" }, y: { type: "number", description: "Y translation" } }, required: ["d"] } },
	{ name: "batch_create", description: "Create many shapes in ONE call. PREFERRED over repeated create_rectangle/create_ellipse/create_line/create_text calls. Each item in the shapes array has a 'type' field ('rectangle' | 'ellipse' | 'line' | 'text') plus type-specific fields (x, y, width, height, radius_x, radius_y, x1, y1, x2, y2, fill_color, stroke_color, stroke_width, text, font_size, corner_radius). All coordinates are in document space. Use this for any drawing with more than a handful of shapes.", inputSchema: { type: "object", properties: { shapes: { type: "array", items: { type: "object" }, description: "Array of shape objects to create" } }, required: ["shapes"] } },
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