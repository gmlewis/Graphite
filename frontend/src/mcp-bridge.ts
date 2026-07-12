// Browser-side MCP bridge: connects to the MCP relay server via WebSocket
// and dispatches incoming JSON-RPC tool calls to the WASM editor.

import type { EditorWrapper } from "/wrapper/pkg/graphite_wasm_wrapper";

const MCP_RELAY_URL = "ws://localhost:8081";
const RECONNECT_INTERVAL = 2000;

// Tool list — must match the tools handled in the WASM wrapper's mcpToolCall
//
// AGENT GUIDANCE (read before using these tools):
//
// 1. Start every session with get_document_info. If it says "No active document",
//    call create_document first. NEVER call list_documents to inspect state — it
//    only refreshes the UI and returns "Document list updated in UI", not a list.
//
// 2. There is a 5-second timeout per tool call. Every call should return in under
//    1 second. If you see timeouts, the browser tab may be stuck or the document
//    has too many layers (see below). Do NOT retry in a tight loop.
//
// 3. ALWAYS prefer import_svg over batch_create over individual create_* calls.
//    - import_svg: ONE call, ONE layer group, unlimited elements. THE BEST TOOL.
//    - batch_create: one call but creates one LAYER PER SHAPE (each shape is a
//      separate InsertSvg internally). With many shapes this creates many layers
//      and slows the browser down. Keep batches to ≤10 shapes, and only when you
//      can't express the artwork as a single SVG.
//    - create_rectangle/ellipse/line/text: one call, one layer. Fine for a few
//      shapes; do NOT use for complex artwork (dozens+ of shapes).
//
// 4. The browser slows down dramatically past ~200 layers. If your document has
//    200+ layers, even simple read calls (get_document_info) may take longer.
//    Prefer import_svg (1 layer per call regardless of element count) to keep the
//    layer count low. If you've accumulated too many layers, create a new document
//    with create_document rather than deleting layers one by one.
//
// 5. Tool call argument size is limited by what you (the agent) can hold in your
//    context window. A single import_svg call with a ~20KB SVG string works well.
//    For larger artwork, split into multiple import_svg calls (each creating a
//    layer group) or use create_path for individual complex curves.
//
// 6. NEVER issue tool calls in parallel. The relay serializes them, but parallel
//    calls interleave responses and cause timeouts. Always call sequentially.
//
// 7. After creating content, call zoom_to_fit so the user can see the result.
//
const TOOLS = [
	{ name: "get_document_info", description: "Get metadata about the active document: name, layer count, artboard count, selected count. ALWAYS CALL THIS FIRST to orient — it tells you whether a document exists and how many layers it has. Do NOT use list_documents, which only refreshes the UI and returns no useful data.", inputSchema: { type: "object", properties: {} } },
	{ name: "create_document", description: "Create a new blank document and make it the active document. Call get_document_info afterwards to confirm. Use this when you need a fresh canvas or when the current document has accumulated too many layers (>200) and is getting slow.", inputSchema: { type: "object", properties: { name: { type: "string" } } } },
	{ name: "get_layer_tree", description: "Get a flat markdown list of all layers in the active document with their integer IDs, kind (layer/group/artboard), name, visibility and lock state. Use this to discover layer IDs for select_layer, get_layer_properties, etc. Note: the list is flat, not nested — it does not show parent/child hierarchy.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_selection", description: "List selected layer IDs (integers) and names in the active document.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_selected_layer_info", description: "Detailed info about every selected layer: ID, kind, name, visibility, lock, bounding box. Richer than get_selection — includes bounding box coordinates. Use this when you need to know where selected layers are positioned.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_layer_properties", description: "Get name, kind, visibility and lock state of a specific layer by its integer ID.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID as string or number, e.g. '42'" } }, required: ["layer_id"] } },
	{ name: "get_layer_bounds", description: "Get the axis-aligned bounding box (min/max XY and size) of a layer by its integer ID. Useful for positioning and layout calculations.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID" } }, required: ["layer_id"] } },
	{ name: "get_node_graph", description: "Dump the document node (implementation + inputs) backing a layer by its integer ID. For advanced inspection of how a layer was constructed.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID" } }, required: ["layer_id"] } },
	{ name: "list_documents", description: "DEPRECATED — do not use. Only refreshes the document list shown in the editor UI. Does NOT return a list of documents; the response is just 'Document list updated in UI'. To inspect the active document, use get_document_info instead.", inputSchema: { type: "object", properties: {} } },
	{ name: "import_svg", description: "Import a complete SVG string as a new layer group. THE MOST POWERFUL AND EFFICIENT TOOL for creating complex artwork — construct one SVG with many elements (<rect>, <circle>, <ellipse>, <path>, <line>, <polygon>, <polyline>, <g>, <text>, <defs>, <linearGradient>, <radialGradient>, etc.) and the editor parses it all in one shot, creating a SINGLE layer group regardless of how many elements the SVG contains. This is far faster and creates far fewer layers than hundreds of individual create_* or batch_create calls. The SVG is placed at document origin (0,0) using absolute coordinates. For very large artwork, split into multiple import_svg calls (each creates one layer group). Practical limit: keep each SVG string under ~20KB for reliable agent-to-relay transmission.", inputSchema: { type: "object", properties: { svg: { type: "string", description: "Complete SVG string with root <svg> element. Must start with '<svg' and contain a closing '</svg>' tag." }, name: { type: "string", description: "Optional name for the imported layer group" } }, required: ["svg"] } },
	{ name: "create_path", description: "Create a vector path from SVG path data (the 'd' attribute). Creates a SINGLE layer — efficient even with hundreds of path points. Optionally fill (hex) and/or stroke (hex + width). x,y translate the path. Use this for individual complex curves, spirographs, mathematical curves, and freeform shapes that don't fit the rectangle/ellipse/line primitives. For multiple curves, prefer import_svg with multiple <path> elements instead of many create_path calls.", inputSchema: { type: "object", properties: { d: { type: "string", description: "SVG path data string, e.g. 'M 0 0 L 100 0 L 100 100 Z'. Supports M, L, l, C, c, Z and all standard SVG path commands." }, fill_color: { type: "string", description: "Fill color as hex, e.g. '#FF0000'. Use 'none' for no fill." }, stroke_color: { type: "string", description: "Stroke color as hex" }, stroke_width: { type: "number", description: "Stroke width in pixels" }, x: { type: "number", description: "X translation applied to the path" }, y: { type: "number", description: "Y translation applied to the path" } }, required: ["d"] } },
	{ name: "batch_create", description: "Create many simple shapes in one call. CAUTION: each shape in the array creates a SEPARATE layer (each is internally a separate InsertSvg dispatch). With many shapes this creates many layers and can slow the browser down — keep batches to ≤10 shapes. For complex artwork (dozens+ of shapes), ALWAYS prefer import_svg instead, which creates a single layer group regardless of element count. Each item has a 'type' field ('rectangle' | 'ellipse' | 'line' | 'text') plus type-specific fields (x, y, width, height, radius_x, radius_y, x1, y1, x2, y2, fill_color, stroke_color, stroke_width, text, font_size, corner_radius). All coordinates are in document space.", inputSchema: { type: "object", properties: { shapes: { type: "array", items: { type: "object" }, description: "Array of shape objects to create. Keep to ≤10 items per call to avoid timeouts, especially when the document already has many layers." } }, required: ["shapes"] } },
	{ name: "create_rectangle", description: "Create a rectangle shape as a single layer. x,y is the top-left corner. fill_color is a hex string like '#FF0000'. corner_radius rounds corners (0 = sharp). For more than a few rectangles, prefer import_svg with <rect> elements instead.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, width: { type: "number" }, height: { type: "number" }, fill_color: { type: "string" }, corner_radius: { type: "number" } }, required: ["x", "y", "width", "height"] } },
	{ name: "create_ellipse", description: "Create an ellipse as a single layer. x,y is the CENTER, not top-left. radius_x/radius_y are the half-widths. fill_color is hex like '#FF0000'. For more than a few ellipses, prefer import_svg with <ellipse> elements instead.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, radius_x: { type: "number" }, radius_y: { type: "number" }, fill_color: { type: "string" } }, required: ["x", "y", "radius_x", "radius_y"] } },
	{ name: "create_line", description: "Create a straight line as a single layer, from (x1,y1) to (x2,y2) with the given stroke color (hex) and width. For many lines, prefer import_svg with <line> elements.", inputSchema: { type: "object", properties: { x1: { type: "number" }, y1: { type: "number" }, x2: { type: "number" }, y2: { type: "number" }, stroke_color: { type: "string" }, stroke_width: { type: "number" } }, required: ["x1", "y1", "x2", "y2"] } },
	{ name: "create_text", description: "Create a text layer at x,y with the given content, font size and hex fill color. Uses the default Graphite font. Each call creates one layer.", inputSchema: { type: "object", properties: { x: { type: "number" }, y: { type: "number" }, text: { type: "string" }, font_size: { type: "number" }, fill_color: { type: "string" } }, required: ["x", "y", "text"] } },
	{ name: "select_layer", description: "Select a single layer by its integer ID. Replaces the current selection. Required before set_stroke, set_fill_color, set_opacity, set_blend_mode, move_layer, or delete_selected.", inputSchema: { type: "object", properties: { layer_id: { type: "string", description: "Integer layer ID as string or number" } }, required: ["layer_id"] } },
	{ name: "delete_selected", description: "Delete all currently selected layers.", inputSchema: { type: "object", properties: {} } },
	{ name: "move_layer", description: "Nudge all selected layers by (dx, dy) document pixels. Requires a selection.", inputSchema: { type: "object", properties: { dx: { type: "number" }, dy: { type: "number" } }, required: ["dx", "dy"] } },
	{ name: "set_stroke", description: "Set stroke color (hex) and width (px) on the first selected layer. Requires a selection — call select_layer first.", inputSchema: { type: "object", properties: { color: { type: "string" }, width: { type: "number" } } } },
	{ name: "set_fill_color", description: "Despite the name, this sets the fill OPACITY of the selected layers (0-100 percent), NOT the fill color. There is no tool to set the fill color of an existing layer via this interface — to make colored shapes, set fill_color in the SVG when creating the shape (via import_svg, create_path, batch_create, or create_rectangle/ellipse).", inputSchema: { type: "object", properties: { opacity: { type: "number", description: "Fill opacity percentage 0-100" } }, required: ["opacity"] } },
	{ name: "set_opacity", description: "Set the layer opacity of the selected layers (0-100 percent). Requires a selection.", inputSchema: { type: "object", properties: { opacity: { type: "number", description: "Opacity percentage 0-100" } }, required: ["opacity"] } },
	{ name: "set_blend_mode", description: "Set the blend mode of the selected layers. NOTE: the current implementation always dispatches Normal regardless of the argument — see editor_wrapper.rs. This is a known limitation.", inputSchema: { type: "object", properties: { blend_mode: { type: "string", description: "Blend mode name: Normal, Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion, Hue, Saturation, Color, Luminosity, etc." } }, required: ["blend_mode"] } },
	{ name: "activate_tool", description: "Activate an editor tool by name. One of: Select, Pen, Path, Line, Rectangle, Ellipse, Freehand, Text, Fill, Gradient, Eyedropper. This changes the active tool in the editor UI but does not simulate drawing — use the create_* and import_svg tools for actual drawing.", inputSchema: { type: "object", properties: { tool: { type: "string" } }, required: ["tool"] } },
	{ name: "zoom_to_fit", description: "Zoom the canvas to fit all layers. ALWAYS call this after creating content so the user can see the result.", inputSchema: { type: "object", properties: {} } },
	{ name: "set_viewport", description: "Set the viewport zoom factor (1.0 = 100%). Only zoom is supported — there is no pan argument.", inputSchema: { type: "object", properties: { zoom: { type: "number" } }, required: ["zoom"] } },
	{ name: "undo", description: "Undo the last editor action.", inputSchema: { type: "object", properties: {} } },
	{ name: "redo", description: "Redo the last undone editor action.", inputSchema: { type: "object", properties: {} } },
	{ name: "get_node_catalog", description: "NOT USEFUL through the browser relay — returns a pointer message only. Node catalog queries require the standalone graphite-mcp-server binary. Do not call this tool for drawing tasks.", inputSchema: { type: "object", properties: { category: { type: "string" }, search: { type: "string" } } } },
	{ name: "get_node_details", description: "NOT USEFUL through the browser relay — returns a pointer message only. Node detail queries require the standalone graphite-mcp-server binary. Do not call this tool for drawing tasks.", inputSchema: { type: "object", properties: { node_id: { type: "string" } }, required: ["node_id"] } },
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