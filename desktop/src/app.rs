use rand::Rng;
use rfd::AsyncFileDialog;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ButtonSource, ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::cef;
use crate::consts::CEF_MESSAGE_LOOP_MAX_ITERATIONS;
use crate::dirs;
use crate::event::{AppEvent, AppEventScheduler};
use crate::persist;
use crate::preferences;
use crate::render::{RenderError, RenderState};
use crate::window::Window;
use crate::wrapper::messages::{DesktopFrontendMessage, DesktopWrapperMessage, InputMessage, MouseKeys, MouseState, Preferences};
use crate::wrapper::{DesktopWrapper, MmapResourceStorage, NodeGraphExecutionResult, WgpuContext, serialize_frontend_messages};

use graphite_editor::messages::tool::utility_types::ToolType;

pub(crate) struct App {
	render_state: Option<RenderState>,
	wgpu_context: WgpuContext,
	window: Option<Window>,
	window_scale: f64,
	window_size: PhysicalSize<u32>,
	window_maximized: bool,
	window_fullscreen: bool,
	window_pending_drag: bool,
	pointer_position: PhysicalPosition<f64>,
	pointer_lock_position: Option<PhysicalPosition<f64>>,
	ui_scale: f64,
	app_event_receiver: Receiver<AppEvent>,
	app_event_scheduler: AppEventScheduler,
	desktop_wrapper: DesktopWrapper,
	cef_context: Box<dyn cef::CefContext>,
	cef_schedule: Option<Instant>,
	cef_view_info_sender: Sender<cef::ViewInfoUpdate>,
	cef_init_successful: bool,
	start_render_sender: SyncSender<()>,
	web_communication_initialized: bool,
	web_communication_startup_buffer: Vec<Vec<u8>>,
	preferences: Preferences,
	launch_documents: Option<Vec<PathBuf>>,
	startup_time: Option<Instant>,
	exiting: Arc<AtomicBool>,
	exit_reason: ExitReason,
	#[cfg(feature = "mcp")]
	mcp_handle: Option<crate::mcp::McpHandle>,
}

impl App {
	pub(crate) fn init() {
		Window::init();
	}

	#[allow(clippy::too_many_arguments)]
	pub(crate) fn new(
		cef_context: Box<dyn cef::CefContext>,
		cef_view_info_sender: Sender<cef::ViewInfoUpdate>,
		wgpu_context: WgpuContext,
		app_event_receiver: Receiver<AppEvent>,
		app_event_scheduler: AppEventScheduler,
		preferences: Preferences,
		launch_documents: Vec<PathBuf>,
		#[cfg(feature = "mcp")] start_mcp: bool,
	) -> Self {
		let ctrlc_app_event_scheduler = app_event_scheduler.clone();
		ctrlc::set_handler(move || {
			tracing::info!("Termination signal received, exiting...");
			ctrlc_app_event_scheduler.schedule(AppEvent::Exit);
		})
		.expect("Error setting Ctrl-C handler");

		let exiting = Arc::new(AtomicBool::new(false));

		let rendering_app_event_scheduler = app_event_scheduler.clone();
		let (start_render_sender, start_render_receiver) = std::sync::mpsc::sync_channel(1);
		let exiting_clone = exiting.clone();
		std::thread::spawn(move || {
			let runtime = tokio::runtime::Runtime::new().unwrap();
			loop {
				let result = runtime.block_on(DesktopWrapper::execute_node_graph());
				rendering_app_event_scheduler.schedule(AppEvent::NodeGraphExecutionResult(result));
				let _ = start_render_receiver.recv_timeout(Duration::from_millis(10));
				if exiting_clone.load(Ordering::Relaxed) {
					break;
				}
			}
		});

		let resource_storage = MmapResourceStorage::new(dirs::app_resources_dir()).expect("Failed to initialize on-disk resource storage");

		// Wake the winit event loop when an editor future completes.
		let wake_scheduler = app_event_scheduler.clone();
		let wake = Arc::new(move || {
			wake_scheduler.schedule(AppEvent::DesktopWrapperMessage(DesktopWrapperMessage::Wake));
		});
		let desktop_wrapper = DesktopWrapper::new(rand::rng().random(), Arc::new(resource_storage), dirs::app_autosave_documents_dir(), wgpu_context.clone(), wake);

		#[cfg(feature = "mcp")]
		let mcp_scheduler = app_event_scheduler.clone();

		Self {
			render_state: None,
			wgpu_context,
			window: None,
			window_scale: 1.,
			window_size: PhysicalSize { width: 0, height: 0 },
			window_maximized: false,
			window_fullscreen: false,
			window_pending_drag: false,
			pointer_position: Default::default(),
			pointer_lock_position: None,
			ui_scale: 1.,
			app_event_receiver,
			app_event_scheduler,
			desktop_wrapper,
			cef_context,
			cef_schedule: Some(Instant::now()),
			cef_view_info_sender,
			cef_init_successful: false,
			start_render_sender,
			web_communication_initialized: false,
			web_communication_startup_buffer: Vec::new(),
			preferences,
			launch_documents: Some(launch_documents),
			startup_time: None,
			exiting,
			exit_reason: ExitReason::Shutdown,
			#[cfg(feature = "mcp")]
			mcp_handle: if start_mcp {
				Some(crate::mcp::start(mcp_scheduler))
			} else {
				None
			},
		}
	}

	pub(crate) fn run(mut self, event_loop: EventLoop) -> ExitReason {
		event_loop.run_app(&mut self).unwrap();
		self.exit_reason
	}

	fn exit(&mut self, reason: Option<ExitReason>) {
		if self.exiting.swap(true, Ordering::Relaxed) {
			return;
		}
		let _ = self.start_render_sender.send(());
		if let Some(reason) = reason {
			self.exit_reason = reason;
		}
		self.app_event_scheduler.schedule(AppEvent::Exit);
	}

	#[cfg(feature = "mcp")]
	fn handle_mcp_tool_call(&mut self, tool_name: &str, args: serde_json::Value) -> Result<Vec<String>, String> {
		use graphite_editor::messages::prelude::*;
		use graphite_editor::messages::input_mapper::utility_types::input_keyboard::Key;
		use graph_craft::document::NodeId;
		use graphene_std::raster::BlendMode;

		let responses = match tool_name {
			"list_documents" => {
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::UpdateOpenDocumentsList))));
				vec!["(document list updated)".to_string()]
			}
			"create_document" => {
				let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::NewDocumentWithName { name }))));
				vec!["(document created)".to_string()]
			}
			"get_layer_tree" => {
				let editor = self.desktop_wrapper.editor();
				match editor.active_document() {
					Some(doc) => {
						let metadata = doc.metadata();
						let network = &doc.network_interface;
						let mut out = String::new();
						out.push_str("# Layer Tree\n\n");
						for layer in metadata.all_layers() {
							let node_id = layer.to_node();
							let name = network.display_name(&node_id, &[]);
							let visible = network.is_visible(&node_id, &[]);
							let locked = network.is_locked(&node_id, &[]);
							let is_layer = network.is_layer(&node_id, &[]);
							let is_artboard = network.is_artboard(&node_id, &[]);
							let kind = if is_artboard { "artboard" } else if is_layer { "layer" } else { "group" };
							let vis = if visible { "" } else { " [hidden]" };
							let lock = if locked { " [locked]" } else { "" };
							out.push_str(&format!("- `{}` {} {}{vis}{lock}\n", node_id.0, kind, name));
						}
						vec![out]
					}
					None => vec!["No active document".to_string()],
				}
			}
			"create_rectangle" => {
				let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.);
				let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.);
				let w = args.get("width").and_then(|v| v.as_f64()).unwrap_or(100.);
				let h = args.get("height").and_then(|v| v.as_f64()).unwrap_or(100.);
				let fill = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("#000000");
				let r = args.get("corner_radius").and_then(|v| v.as_f64()).unwrap_or(0.);
				let svg = format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"/>"#);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::InsertSvg {
					name: Some("Rectangle".into()),
					svg,
					mouse: None,
					parent_and_insert_index: None,
					place_at_origin: false,
				})))));
				vec![format!("Created rectangle at ({x}, {y}) {w}x{h}")]
			}
			"create_ellipse" => {
				let cx = args.get("x").and_then(|v| v.as_f64()).unwrap_or(50.);
				let cy = args.get("y").and_then(|v| v.as_f64()).unwrap_or(50.);
				let rx = args.get("radius_x").and_then(|v| v.as_f64()).unwrap_or(50.);
				let ry = args.get("radius_y").and_then(|v| v.as_f64()).unwrap_or(50.);
				let fill = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("#000000");
				let svg = format!(r#"<ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}" fill="{fill}"/>"#);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::InsertSvg {
					name: Some("Ellipse".into()),
					svg,
					mouse: None,
					parent_and_insert_index: None,
					place_at_origin: false,
				})))));
				vec![format!("Created ellipse at ({cx}, {cy}) rx={rx} ry={ry}")]
			}
			"create_line" => {
				let x1 = args.get("x1").and_then(|v| v.as_f64()).unwrap_or(0.);
				let y1 = args.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.);
				let x2 = args.get("x2").and_then(|v| v.as_f64()).unwrap_or(100.);
				let y2 = args.get("y2").and_then(|v| v.as_f64()).unwrap_or(100.);
				let stroke = args.get("stroke_color").and_then(|v| v.as_str()).unwrap_or("#000000");
				let sw = args.get("stroke_width").and_then(|v| v.as_f64()).unwrap_or(2.);
				let svg = format!(r#"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{stroke}" stroke-width="{sw}"/>"#);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::InsertSvg {
					name: Some("Line".into()),
					svg,
					mouse: None,
					parent_and_insert_index: None,
					place_at_origin: false,
				})))));
				vec![format!("Created line ({x1},{y1}) to ({x2},{y2})")]
			}
			"create_text" => {
				let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.);
				let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.);
				let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("Text");
				let font_size = args.get("font_size").and_then(|v| v.as_f64()).unwrap_or(24.);
				let fill = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("#000000");
				let svg = format!(r#"<text x="{x}" y="{y}" font-size="{font_size}" fill="{fill}">{text}</text>"#);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::InsertSvg {
					name: Some("Text".into()),
					svg,
					mouse: None,
					parent_and_insert_index: None,
					place_at_origin: false,
				})))));
				vec![format!("Created text at ({x}, {y}): \"{text}\"")]
			}
			"select_layer" => {
				let layer_id_str = args.get("layer_id").and_then(|v| v.as_str()).ok_or("Missing layer_id")?;
				let id_num: u64 = layer_id_str.parse().map_err(|e| format!("Invalid layer_id '{layer_id_str}': {e}"))?;
				let id = NodeId(id_num);
				let ctrl = args.get("clear_existing").and_then(|v| v.as_bool()).unwrap_or(true);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SelectLayer {
					id,
					ctrl: false,
					shift: false,
				})))));
				if ctrl {
					// Also deselect others first by selecting without ctrl/shift after a deselect
					// Actually, SelectLayer with ctrl=false, shift=false replaces the selection
				}
				vec![format!("Selected layer {id_num}")]
			}
			"delete_selected" => {
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DeleteSelectedLayers)))));
				vec!["(selected layers deleted)".to_string()]
			}
			"undo" => {
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DocumentHistoryBackward)))));
				vec!["(undone)".to_string()]
			}
			"redo" => {
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DocumentHistoryForward)))));
				vec!["(redone)".to_string()]
			}
			"set_fill_color" => {
				let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(100.);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SetFillForSelectedLayers {
					fill: opacity / 100.,
				})))));
				vec![format!("Set fill opacity to {opacity}%")]
			}
			"set_stroke" => {
				let color_str = args.get("color").and_then(|v| v.as_str()).unwrap_or("#000000");
				let weight = args.get("width").and_then(|v| v.as_f64()).unwrap_or(2.);
				let hex = color_str.trim().trim_start_matches('#');
				let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| format!("Invalid color hex: {e}"))? as f32 / 255.;
				let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| format!("Invalid color hex: {e}"))? as f32 / 255.;
				let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| format!("Invalid color hex: {e}"))? as f32 / 255.;
				let a = if hex.len() >= 8 { u8::from_str_radix(&hex[6..8], 16).map_err(|e| format!("Invalid color hex: {e}"))? as f32 / 255. } else { 1. };
				let color = graphene_std::Color::from_rgbaf32_unchecked(r, g, b, a);
				let editor = self.desktop_wrapper.editor();
				let selected_layer = editor.active_document()
					.and_then(|doc| {
						let metadata = doc.metadata();
						doc.network_interface.selected_nodes().selected_layers(metadata).next()
					});
				match selected_layer {
					Some(layer) => {
						let stroke = graphene_std::vector::style::Stroke {
							weight,
							..Default::default()
						};
						let _ = editor;
						self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::GraphOperation(GraphOperationMessage::StrokeSet {
							layer,
							color: Some(color),
							stroke,
						}))))));
						vec![format!("Set stroke on layer: weight={weight}, color={color_str}")]
					}
					None => vec!["No layer selected".to_string()],
				}
			}
			"set_opacity" => {
				let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(100.);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SetOpacityForSelectedLayers {
					opacity: opacity / 100.,
				})))));
				vec![format!("Set opacity to {opacity}%")]
			}
			"set_blend_mode" => {
				let mode_str = args.get("blend_mode").and_then(|v| v.as_str()).unwrap_or("Normal");
				let blend_mode = match mode_str {
					"Normal" => BlendMode::Normal,
					"Darken" => BlendMode::Darken,
					"Multiply" => BlendMode::Multiply,
					"ColorBurn" => BlendMode::ColorBurn,
					"LinearBurn" => BlendMode::LinearBurn,
					"DarkerColor" => BlendMode::DarkerColor,
					"Lighten" => BlendMode::Lighten,
					"Screen" => BlendMode::Screen,
					"ColorDodge" => BlendMode::ColorDodge,
					"LinearDodge" => BlendMode::LinearDodge,
					"LighterColor" => BlendMode::LighterColor,
					"Overlay" => BlendMode::Overlay,
					"SoftLight" => BlendMode::SoftLight,
					"HardLight" => BlendMode::HardLight,
					"VividLight" => BlendMode::VividLight,
					"LinearLight" => BlendMode::LinearLight,
					"PinLight" => BlendMode::PinLight,
					"HardMix" => BlendMode::HardMix,
					"Difference" => BlendMode::Difference,
					"Exclusion" => BlendMode::Exclusion,
					"Subtract" => BlendMode::Subtract,
					"Divide" => BlendMode::Divide,
					"Hue" => BlendMode::Hue,
					"Saturation" => BlendMode::Saturation,
					"Color" => BlendMode::Color,
					"Luminosity" => BlendMode::Luminosity,
					_ => return Err(format!("Unknown blend mode: {mode_str}")),
				};
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SetBlendModeForSelectedLayers {
					blend_mode,
				})))));
				vec![format!("Set blend mode to {mode_str}")]
			}
			"move_layer" => {
				let dx = args.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.);
				let dy = args.get("dy").and_then(|v| v.as_f64()).unwrap_or(0.);
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::NudgeSelectedLayers {
					delta_x: dx,
					delta_y: dy,
					resize: Key::Alt,
					resize_opposite: Key::Control,
				})))));
				vec![format!("Moved selected layers by ({dx}, {dy})")]
			}
			"get_selection" => {
				let editor = self.desktop_wrapper.editor();
				match editor.active_document() {
					Some(doc) => {
						let metadata = doc.metadata();
						let selected = doc.network_interface.selected_nodes();
						let layers: Vec<String> = selected.selected_layers(metadata)
							.map(|l| {
								let node_id = l.to_node();
								let name = doc.network_interface.display_name(&node_id, &[]);
								format!("{} ({})", node_id.0, name)
							})
							.collect();
						if layers.is_empty() {
							vec!["No layers selected".to_string()]
						} else {
							vec![format!("Selected layers ({}):\n{}", layers.len(), layers.join("\n"))]
						}
					}
					None => vec!["No active document".to_string()],
				}
			}
			"get_layer_properties" => {
				let layer_id_str = args.get("layer_id").and_then(|v| v.as_str()).ok_or("Missing layer_id")?;
				let id_num: u64 = layer_id_str.parse().map_err(|e| format!("Invalid layer_id '{layer_id_str}': {e}"))?;
				let node_id = NodeId(id_num);
				let editor = self.desktop_wrapper.editor();
				match editor.active_document() {
					Some(doc) => {
						let network = &doc.network_interface;
						let name = network.display_name(&node_id, &[]);
						let visible = network.is_visible(&node_id, &[]);
						let locked = network.is_locked(&node_id, &[]);
						let is_layer = network.is_layer(&node_id, &[]);
						let is_artboard = network.is_artboard(&node_id, &[]);
						let kind = if is_artboard { "artboard" } else if is_layer { "layer" } else { "group" };
						let mut out = format!("# Layer Properties: {name}\n\n");
						out.push_str(&format!("- **ID:** `{}`\n", node_id.0));
						out.push_str(&format!("- **Kind:** {kind}\n"));
						out.push_str(&format!("- **Visible:** {visible}\n"));
						out.push_str(&format!("- **Locked:** {locked}\n"));
						vec![out]
					}
					None => vec!["No active document".to_string()],
				}
			}
			"get_node_graph" => {
				let layer_id_str = args.get("layer_id").and_then(|v| v.as_str()).ok_or("Missing layer_id")?;
				let id_num: u64 = layer_id_str.parse().map_err(|e| format!("Invalid layer_id '{layer_id_str}': {e}"))?;
				let node_id = NodeId(id_num);
				let editor = self.desktop_wrapper.editor();
				match editor.active_document() {
					Some(doc) => {
						let network = &doc.network_interface;
						let name = network.display_name(&node_id, &[]);
						match network.document_node(&node_id, &[]) {
							Some(node) => {
								let mut out = format!("# Node Graph: {name}\n\n");
								out.push_str(&format!("**Node ID:** `{}`\n", node_id.0));
								out.push_str(&format!("**Implementation:** {:?}\n", node.implementation));
								out.push_str(&format!("**Inputs:** {}\n", node.inputs.len()));
								for (i, input) in node.inputs.iter().enumerate() {
									out.push_str(&format!("  - Input {i}: {input:?}\n"));
								}
								vec![out]
							}
							None => vec![format!("Node {id_num} not found")],
						}
					}
					None => vec!["No active document".to_string()],
				}
			}
			"activate_tool" => {
				let tool = args.get("tool").and_then(|v| v.as_str()).unwrap_or("Select");
				let tool_type = match tool {
					"Select" => ToolType::Select,
					"Pen" => ToolType::Pen,
					"Path" => ToolType::Path,
					"Line" => ToolType::Line,
					"Rectangle" => ToolType::Rectangle,
					"Ellipse" => ToolType::Ellipse,
					"Polygon" => ToolType::Shape,
					"Star" => ToolType::Shape,
					"Spiral" => ToolType::Shape,
					"Freehand" => ToolType::Freehand,
					"Text" => ToolType::Text,
					"Fill" => ToolType::Fill,
					"Gradient" => ToolType::Gradient,
					"Eyedropper" => ToolType::Eyedropper,
					_ => ToolType::Select,
				};
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Tool(ToolMessage::ActivateTool { tool_type }))));
				vec![format!("Activated tool: {tool}")]
			}
			"zoom_to_fit" => {
				self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::ZoomCanvasToFitAll)))));
				vec!["(zoomed to fit)".to_string()]
			}
			"set_viewport" => {
				let zoom = args.get("zoom").and_then(|v| v.as_f64());
				if let Some(zf) = zoom {
					let msg = Message::Portfolio(PortfolioMessage::Document(
						DocumentMessage::Navigation(NavigationMessage::CanvasZoomSet { zoom_factor: zf })
					));
					self.desktop_wrapper.dispatch(DesktopWrapperMessage::FromWeb(Box::new(msg)));
				}
				vec!["(set_viewport partially implemented — zoom only)".to_string()]
			}
			"get_node_catalog" | "get_node_details" => {
				match graphite_mcp_server::tools::call_tool(tool_name, args) {
					Ok(contents) => contents.into_iter().filter_map(|c| match c {
						graphite_mcp_server::mcp_protocol::ToolContent::Text { text } => Some(text),
						_ => None,
					}).collect(),
					Err(e) => vec![format!("Error: {e}")],
				}
			}
			_ => {
				vec![format!("Unknown tool: {tool_name}")]
			}
		};

		Ok(responses)
	}

	fn resize(&mut self) {
		let Some(window) = &self.window else {
			tracing::error!("Resize failed due to missing window");
			return;
		};

		let maximized = window.is_maximized();
		if maximized != self.window_maximized {
			self.window_maximized = maximized;
			self.app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(DesktopWrapperMessage::UpdateMaximized { maximized }));
		}

		let fullscreen = window.is_fullscreen();
		if fullscreen != self.window_fullscreen {
			self.window_fullscreen = fullscreen;
			self.app_event_scheduler
				.schedule(AppEvent::DesktopWrapperMessage(DesktopWrapperMessage::UpdateFullscreen { fullscreen }));
		}

		let size = window.surface_size();
		let scale = window.scale_factor() * self.ui_scale;
		let is_new_size = size != self.window_size;
		let is_new_scale = scale != self.window_scale;

		if !is_new_size && !is_new_scale {
			return;
		}

		if is_new_size {
			let _ = self.cef_view_info_sender.send(cef::ViewInfoUpdate::Size {
				width: size.width,
				height: size.height,
			});
		}
		if is_new_scale {
			let _ = self.cef_view_info_sender.send(cef::ViewInfoUpdate::Scale(scale));
		}

		self.cef_context.notify_view_info_changed();

		if let Some(render_state) = &mut self.render_state {
			render_state.resize(size.width, size.height);
		}

		window.request_redraw();

		self.window_size = size;
		self.window_scale = scale;
	}

	fn handle_desktop_frontend_message(&mut self, message: DesktopFrontendMessage, responses: &mut Vec<DesktopWrapperMessage>) {
		match message {
			DesktopFrontendMessage::ToWeb(messages) => {
				let Some(bytes) = serialize_frontend_messages(messages) else {
					tracing::error!("Failed to serialize frontend messages");
					return;
				};
				self.send_or_queue_web_message(bytes);
			}
			DesktopFrontendMessage::OpenFileDialog { title, filters, multiple, context } => {
				let app_event_scheduler = self.app_event_scheduler.clone();
				let _ = thread::spawn(move || {
					let mut dialog = AsyncFileDialog::new().set_title(title);
					for filter in filters {
						dialog = dialog.add_filter(filter.name, &filter.extensions);
					}

					let handles = if multiple {
						futures::executor::block_on(dialog.pick_files()).unwrap_or_default()
					} else {
						futures::executor::block_on(dialog.pick_file()).into_iter().collect()
					};

					for handle in handles {
						let path = handle.path().to_path_buf();
						match fs::read(&path) {
							Ok(content) => {
								let message = DesktopWrapperMessage::FileDialogResult { path, content, context };
								app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
							}
							Err(e) => tracing::error!("Failed to read file {}: {}", path.display(), e),
						}
					}
				});
			}
			DesktopFrontendMessage::SaveFileDialog {
				title,
				default_filename,
				default_folder,
				filters,
				context,
			} => {
				let app_event_scheduler = self.app_event_scheduler.clone();
				let _ = thread::spawn(move || {
					let mut dialog = AsyncFileDialog::new().set_title(title).set_file_name(default_filename);
					if let Some(folder) = default_folder {
						dialog = dialog.set_directory(folder);
					}
					for filter in filters {
						dialog = dialog.add_filter(filter.name, &filter.extensions);
					}

					let show_dialog = async move { dialog.save_file().await.map(|f| f.path().to_path_buf()) };

					if let Some(path) = futures::executor::block_on(show_dialog) {
						let message = DesktopWrapperMessage::SaveFileDialogResult { path, context };
						app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
					}
				});
			}
			DesktopFrontendMessage::WriteFile { path, content } => {
				if let Err(e) = fs::write(&path, content) {
					tracing::error!("Failed to write file {}: {}", path.display(), e);
				}
			}
			DesktopFrontendMessage::OpenUrl(url) => {
				let _ = thread::spawn(move || {
					if let Err(e) = open::that(&url) {
						tracing::error!("Failed to open URL: {}: {}", url, e);
					}
				});
			}
			DesktopFrontendMessage::UpdateViewportPhysicalBounds { x, y, width, height } => {
				if let Some(render_state) = &mut self.render_state
					&& let Some(window) = &self.window
				{
					let window_size = window.surface_size();

					let viewport_offset_x = x / window_size.width as f64;
					let viewport_offset_y = y / window_size.height as f64;
					render_state.set_viewport_offset([viewport_offset_x as f32, viewport_offset_y as f32]);

					let viewport_scale_x = if width != 0. { window_size.width as f64 / width } else { 1. };
					let viewport_scale_y = if height != 0. { window_size.height as f64 / height } else { 1. };
					render_state.set_viewport_scale([viewport_scale_x as f32, viewport_scale_y as f32]);
				}
			}
			DesktopFrontendMessage::UpdateUIScale { scale } => {
				self.ui_scale = scale;
				self.resize();
			}
			DesktopFrontendMessage::UpdateOverlays(scene) => {
				if let Some(render_state) = &mut self.render_state {
					render_state.set_overlays_scene(scene);
				}
				if let Some(window) = &self.window {
					window.request_redraw();
				}
			}
			DesktopFrontendMessage::PersistenceWriteState { state } => {
				persist::write_state(state);
			}
			DesktopFrontendMessage::PersistenceReadState => {
				responses.push(DesktopWrapperMessage::LoadPersistedState { state: persist::read_state() });
			}
			DesktopFrontendMessage::PersistenceReadDocument { id } => {
				if let Some(document) = persist::read_document_content(&id) {
					responses.push(DesktopWrapperMessage::LoadDocumentContent { id, document });
				} else {
					tracing::error!("Failed to read document content for {id:?}");
				}
			}
			DesktopFrontendMessage::PersistenceWriteDocument { id, document_serialized_content } => {
				persist::write_document_content(id, document_serialized_content);
			}
			DesktopFrontendMessage::PersistenceDeleteDocument { id } => {
				persist::delete_document(&id);
			}
			DesktopFrontendMessage::PersistenceWritePreferences { preferences } => {
				preferences::write(preferences);
			}
			DesktopFrontendMessage::PersistenceLoadPreferences => {
				let preferences = preferences::read();
				let message = DesktopWrapperMessage::LoadPreferences { preferences };
				responses.push(message);
			}
			DesktopFrontendMessage::OpenLaunchDocuments => {
				let Some(launch_documents) = std::mem::take(&mut self.launch_documents) else {
					tracing::error!("OpenLaunchDocuments should only be sent once");
					return;
				};
				self.app_event_scheduler.schedule(AppEvent::OpenFiles(launch_documents));
			}
			DesktopFrontendMessage::UpdateMenu { entries } => {
				if let Some(window) = &self.window {
					window.update_menu(entries);
				}
			}
			DesktopFrontendMessage::ClipboardRead => {
				if let Some(window) = &self.window {
					let content = window.clipboard_read();
					let message = DesktopWrapperMessage::ClipboardReadResult { content };
					self.app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
				}
			}
			DesktopFrontendMessage::ClipboardWrite { content } => {
				if let Some(window) = &mut self.window {
					window.clipboard_write(content);
				}
			}
			DesktopFrontendMessage::PointerLock => {
				self.pointer_lock_position = Some(self.pointer_position);
				if let Some(window) = &self.window {
					window.start_pointer_lock();
				}
			}
			DesktopFrontendMessage::WindowClose => {
				self.app_event_scheduler.schedule(AppEvent::Exit);
			}
			DesktopFrontendMessage::WindowMinimize => {
				if let Some(window) = &self.window {
					window.minimize();
				}
			}
			DesktopFrontendMessage::WindowMaximize => {
				if let Some(window) = &self.window {
					window.toggle_maximize();
				}
			}
			DesktopFrontendMessage::WindowFullscreen => {
				if let Some(window) = &mut self.window {
					window.toggle_fullscreen();
				}
			}
			DesktopFrontendMessage::WindowDrag => {
				self.window_pending_drag = true;
			}
			DesktopFrontendMessage::WindowFocus => {
				if let Some(window) = &self.window {
					window.focus();
				}
			}
			DesktopFrontendMessage::WindowHide => {
				if let Some(window) = &self.window {
					window.hide();
				}
			}
			DesktopFrontendMessage::WindowHideOthers => {
				if let Some(window) = &self.window {
					window.hide_others();
				}
			}
			DesktopFrontendMessage::WindowShowAll => {
				if let Some(window) = &self.window {
					window.show_all();
				}
			}
			DesktopFrontendMessage::Restart => {
				self.exit(Some(ExitReason::Restart));
			}
			DesktopFrontendMessage::LoadThirdPartyLicenses => {
				let compressed = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/third-party-licenses.txt.xz"));
				let mut reader = lzma_rust2::XzReader::new(compressed.as_slice(), false);
				let mut text = String::new();
				if let Err(e) = reader.read_to_string(&mut text) {
					tracing::error!("Failed to decompress third-party licenses: {e}");
					return;
				}

				let message = DesktopWrapperMessage::LoadThirdPartyLicenses { text };
				responses.push(message);
			}
		}
	}

	fn handle_desktop_frontend_messages(&mut self, messages: Vec<DesktopFrontendMessage>) {
		let mut responses = Vec::new();
		for message in messages {
			self.handle_desktop_frontend_message(message, &mut responses);
		}
		for message in responses {
			self.dispatch_desktop_wrapper_message(message);
		}
	}

	fn dispatch_desktop_wrapper_message(&mut self, message: DesktopWrapperMessage) {
		let responses = self.desktop_wrapper.dispatch(message);
		self.handle_desktop_frontend_messages(responses);
	}

	fn send_or_queue_web_message(&mut self, message: Vec<u8>) {
		if self.web_communication_initialized {
			self.cef_context.send_web_message(message);
		} else {
			self.web_communication_startup_buffer.push(message);
		}
	}

	fn user_event(&mut self, event_loop: &dyn ActiveEventLoop, event: AppEvent) {
		match event {
			AppEvent::WebCommunicationInitialized => {
				self.web_communication_initialized = true;
				for message in self.web_communication_startup_buffer.drain(..) {
					self.cef_context.send_web_message(message);
				}
			}
			AppEvent::DesktopWrapperMessage(message) => self.dispatch_desktop_wrapper_message(message),
			AppEvent::NodeGraphExecutionResult(result) => match result {
				NodeGraphExecutionResult::HasRun(texture) => {
					self.dispatch_desktop_wrapper_message(DesktopWrapperMessage::PollNodeGraphEvaluation);
					if let Some(texture) = texture
						&& let Some(render_state) = self.render_state.as_mut()
						&& let Some(window) = self.window.as_ref()
					{
						render_state.bind_viewport_texture(texture);
						window.request_redraw();
					}
				}
				NodeGraphExecutionResult::NotRun => {}
			},
			AppEvent::UiUpdate(texture) => {
				if let Some(render_state) = self.render_state.as_mut() {
					render_state.bind_ui_texture(texture);
				}
				if let Some(window) = &self.window {
					window.request_redraw();
				}
				if !self.cef_init_successful {
					self.cef_init_successful = true;
				}
			}
			AppEvent::ScheduleBrowserWork(instant) => {
				if instant <= Instant::now() {
					self.cef_context.work();
				} else {
					self.cef_schedule = Some(instant);
				}
			}
			AppEvent::CursorChange(cursor) => {
				if let Some(window) = &mut self.window {
					window.set_cursor(event_loop, cursor);
				}
			}
			AppEvent::Exit => {
				tracing::info!("Exiting main event loop");
				event_loop.exit();
			}
			AppEvent::OpenFiles(paths) => {
				// Accumulate launch documents until OpenLaunchDocuments message is received
				if let Some(launch_documents) = &mut self.launch_documents {
					launch_documents.extend(paths);
					return;
				}

				if paths.is_empty() {
					return;
				}
				let app_event_scheduler = self.app_event_scheduler.clone();
				let _ = thread::spawn(move || {
					for path in paths {
						tracing::info!("Opening file: {}", path.display());
						if let Ok(content) = fs::read(&path) {
							let message = DesktopWrapperMessage::OpenFile { path, content };
							app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
						} else {
							tracing::error!("Failed to read file: {}", path.display());
						}
					}
				});
			}
			#[cfg(target_os = "macos")]
			AppEvent::MenuEvent { id } => {
				self.dispatch_desktop_wrapper_message(DesktopWrapperMessage::MenuEvent { id });
			}
			#[cfg(feature = "mcp")]
			AppEvent::McpToolCall { tool_name, args, response_sender } => {
				let result = self.handle_mcp_tool_call(&tool_name, args);
				let _ = response_sender.send(result);
			}
		}
	}
}
impl ApplicationHandler for App {
	fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
		let window = Window::new(event_loop, self.app_event_scheduler.clone());
		self.window = Some(window);

		#[cfg(not(target_os = "macos"))]
		let present_mode = None;
		#[cfg(target_os = "macos")]
		let present_mode = if !self.preferences.vsync { Some(wgpu::PresentMode::Immediate) } else { None };

		let render_state = RenderState::new(self.window.as_ref().unwrap(), self.wgpu_context.clone(), present_mode);
		self.render_state = Some(render_state);

		if let Some(window) = &self.window.as_ref() {
			window.show();
		}

		self.resize();

		self.startup_time = Some(Instant::now());
	}

	fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
		while let Ok(event) = self.app_event_receiver.try_recv() {
			self.user_event(event_loop, event);
		}
	}

	fn window_event(&mut self, _event_loop: &dyn ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
		// Handle pointer lock release
		if let Some(pointer_lock_position) = self.pointer_lock_position
			&& let WindowEvent::PointerButton {
				state: ElementState::Released,
				button: ButtonSource::Mouse(MouseButton::Left),
				..
			} = event
		{
			self.pointer_lock_position = None;
			if let Some(window) = &self.window {
				window.end_pointer_lock();
			}
			self.cef_context.handle_window_event(&WindowEvent::PointerMoved {
				device_id: None,
				position: pointer_lock_position,
				primary: true,
				source: winit::event::PointerSource::Mouse,
			});
		}

		self.cef_context.handle_window_event(&event);

		match event {
			WindowEvent::CloseRequested => {
				self.app_event_scheduler.schedule(AppEvent::Exit);
			}
			WindowEvent::SurfaceResized(_) | WindowEvent::ScaleFactorChanged { .. } => {
				self.resize();
			}
			WindowEvent::RedrawRequested => {
				#[cfg(target_os = "macos")]
				self.resize();

				let Some(render_state) = &mut self.render_state else { return };
				if let Some(window) = &self.window {
					if !window.can_render() {
						return;
					}

					match render_state.render(window) {
						Ok(_) => {}
						Err(RenderError::OutdatedUITextureError) => {
							self.cef_context.notify_view_info_changed();
						}
						Err(RenderError::SurfaceLost) => {
							tracing::warn!("lost surface");
						}
						Err(other) => tracing::error!("Render error: {:?}", other),
					}
					let _ = self.start_render_sender.try_send(());
				}

				if !self.cef_init_successful
					&& !self.preferences.disable_ui_acceleration
					&& self.web_communication_initialized
					&& let Some(startup_time) = self.startup_time
					&& startup_time.elapsed() > Duration::from_secs(3)
				{
					tracing::error!("UI acceleration not working, exiting.");
					self.exit(Some(ExitReason::UiAccelerationFailure));
				}
			}
			WindowEvent::DragDropped { paths, .. } => {
				for path in paths {
					match fs::read(&path) {
						Ok(content) => {
							let message = DesktopWrapperMessage::ImportFile { path, content };
							self.app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
						}
						Err(e) => {
							tracing::error!("Failed to read dropped file {}: {}", path.display(), e);
							return;
						}
					};
				}
			}

			// Forward and Back buttons are not supported by CEF and thus need to be directly forwarded the editor
			WindowEvent::PointerButton {
				button: ButtonSource::Mouse(button),
				state: ElementState::Pressed,
				..
			} => {
				let mouse_keys = match button {
					MouseButton::Back => Some(MouseKeys::BACK),
					MouseButton::Forward => Some(MouseKeys::FORWARD),
					_ => None,
				};
				if let Some(mouse_keys) = mouse_keys {
					let message = DesktopWrapperMessage::Input(InputMessage::PointerDown {
						editor_mouse_state: MouseState { mouse_keys, ..Default::default() },
						modifier_keys: Default::default(),
					});
					self.app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));

					let message = DesktopWrapperMessage::Input(InputMessage::PointerUp {
						editor_mouse_state: Default::default(),
						modifier_keys: Default::default(),
					});
					self.app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
				}
			}

			WindowEvent::PointerMoved { position, .. } | WindowEvent::PointerLeft { position: Some(position), .. } | WindowEvent::PointerEntered { position, .. }
				if self.pointer_lock_position.is_none() =>
			{
				self.pointer_position = position;

				if self.window_pending_drag {
					self.window_pending_drag = false;
					if let Some(window) = &self.window {
						window.start_drag();
					}
				}
			}

			WindowEvent::PointerButton {
				button: ButtonSource::Mouse(MouseButton::Left),
				state: ElementState::Released,
				..
			} => {
				self.window_pending_drag = false;
			}

			_ => {}
		}

		// Notify cef of possible input events
		self.cef_context.work();
	}

	fn device_event(&mut self, _event_loop: &dyn ActiveEventLoop, _device_id: Option<winit::event::DeviceId>, event: winit::event::DeviceEvent) {
		if self.pointer_lock_position.is_some()
			&& let winit::event::DeviceEvent::PointerMotion { delta: (x, y) } = event
		{
			let message = DesktopWrapperMessage::PointerLockMove { x, y };
			self.app_event_scheduler.schedule(AppEvent::DesktopWrapperMessage(message));
		}
	}

	fn new_events(&mut self, _event_loop: &dyn ActiveEventLoop, cause: winit::event::StartCause) {
		if let StartCause::ResumeTimeReached { .. } = cause
			&& let Some(window) = &self.window
		{
			window.request_redraw();
		}
	}

	fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
		// Set a timeout in case we miss any cef schedule requests
		let mut wait_until = Instant::now() + Duration::from_millis(10);
		if let Some(schedule) = self.cef_schedule
			&& schedule < Instant::now()
		{
			self.cef_schedule = None;
			// Poll cef message loop multiple times to avoid message loop starvation
			for _ in 0..CEF_MESSAGE_LOOP_MAX_ITERATIONS {
				self.cef_context.work();
			}
		} else if let Some(cef_schedule) = self.cef_schedule {
			wait_until = wait_until.min(cef_schedule);
		}
		event_loop.set_control_flow(ControlFlow::WaitUntil(wait_until));
	}
}

pub(crate) enum ExitReason {
	Shutdown,
	Restart,
	UiAccelerationFailure,
}
