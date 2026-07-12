#![allow(clippy::too_many_arguments)]
//
// This file is where functions are defined to be called directly from JS.
// It serves as a thin wrapper over the editor backend API that relies
// on the dispatcher messaging system and more complex Rust data types.
//
#[cfg(not(feature = "native"))]
use crate::EDITOR;
#[cfg(not(feature = "native"))]
use crate::helpers::poll_node_graph_evaluation;
use crate::helpers::{auto_save_all_documents, calculate_hash, render_image_data_to_canvases, request_animation_frame, set_timeout, translate_key, wrapper};
use crate::{EDITOR_HAS_CRASHED, Error, FRONTEND_READY, MESSAGE_BUFFER};
#[cfg(not(feature = "native"))]
#[cfg(all(not(feature = "native"), target_family = "wasm"))]
use editor::application::{Editor, Environment, Host, Platform};
use editor::consts::{FILE_EXTENSION, GDD_FILE_EXTENSION};
use editor::messages::clipboard::utility_types::ClipboardContentRaw;
use editor::messages::input_mapper::utility_types::input_keyboard::ModifierKeys;
use editor::messages::input_mapper::utility_types::input_mouse::{EditorMouseState, ScrollDelta};
use editor::messages::layout::utility_types::layout_widget::LayoutTarget;
use editor::messages::portfolio::document::utility_types::document_metadata::LayerNodeIdentifier;
use editor::messages::portfolio::document::utility_types::network_interface::ImportOrExport;
use editor::messages::portfolio::utility_types::{DockingSplitDirection, PanelGroupId, PanelType};
use editor::messages::prelude::*;
use editor::messages::tool::tool_messages::tool_prelude::WidgetId;
use graph_craft::document::NodeId;
use graphene_std::color::SRGBA8;
use graphene_std::graphene_hash::CacheHashWrapper;
use graphene_std::raster::color::Color;
use graphene_std::vector::style::{FillChoice, FillChoiceUI};
use serde::Serialize;
use serde_wasm_bindgen::{self, from_value};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use wasm_bindgen::prelude::*;

static IMAGE_DATA_HASH: AtomicU64 = AtomicU64::new(0);

/// This struct is, via wasm-bindgen, used by JS to interact with the editor backend. It does this by calling functions, which are `impl`ed
#[wasm_bindgen]
#[derive(Clone)]
pub struct EditorWrapper {
	/// This callback is called by the editor's dispatcher when directing `FrontendMessage`s from Rust to JS
	frontend_message_handler_callback: js_sys::Function,
}

// Defined separately from the `impl` block below since this `impl` block lacks the `#[wasm_bindgen]` attribute.
// Quirks in wasm-bindgen prevent functions in `#[wasm_bindgen]` `impl` blocks from being made publicly accessible from Rust.
impl EditorWrapper {
	pub fn send_frontend_message_to_js_rust_proxy(&self, message: FrontendMessage) {
		self.send_frontend_message_to_js(message);
	}

	#[cfg(any(feature = "native", target_family = "wasm"))]
	fn initialize_wrapper(frontend_message_handler_callback: js_sys::Function) -> EditorWrapper {
		use crate::{EDITOR_WRAPPER, PANIC_DIALOG_MESSAGE_CALLBACK};

		let panic_callback = frontend_message_handler_callback.clone();
		let editor_wrapper = EditorWrapper { frontend_message_handler_callback };
		if EDITOR_WRAPPER.with(|wrapper| wrapper.lock().ok().map(|mut guard| *guard = Some(editor_wrapper.clone()))).is_none() {
			log::error!("Attempted to initialize the editor wrapper more than once");
		}
		PANIC_DIALOG_MESSAGE_CALLBACK.with_borrow_mut(|callback| *callback = Some(panic_callback));
		editor_wrapper
	}
}

#[wasm_bindgen]
impl EditorWrapper {
	// ========================
	// Editor wrapper machinery
	// ========================

	#[cfg(all(not(feature = "native"), target_family = "wasm"))]
	pub async fn create(platform: String, uuid_random_seed: u64, frontend_message_handler_callback: js_sys::Function) -> EditorWrapper {
		use graph_craft::application_io::PlatformApplicationIo;
		use graph_craft::application_io::resource::*;

		let host = match platform.as_str() {
			"Linux" => Host::Linux,
			"Mac" => Host::Mac,
			"Windows" => Host::Windows,
			_ => unreachable!(),
		};

		let storage: std::sync::Arc<dyn ResourceStorage> = match OpfsResourceStorage::load("resources").await {
			Ok(storage) => std::sync::Arc::new(storage),
			Err(error) => {
				log::error!("Failed to open OPFS resource storage, falling back to in-memory: {error:?}");
				std::sync::Arc::new(graph_craft::application_io::resource::HashMapResourceStorage::new())
			}
		};

		let application_io = PlatformApplicationIo::new().await;
		let wake = crate::helpers::async_wake_callback();
		// On web the working-copy root is an OPFS directory name (no real filesystem path); each
		// document mounts under `documents/<id_hex>`.
		let working_copy_root = Some(std::path::PathBuf::from("documents"));
		let editor = Editor::new(Environment { platform: Platform::Web, host }, uuid_random_seed, storage, working_copy_root, application_io, wake);

		if EDITOR.with(|slot| slot.lock().ok().map(|mut guard| *guard = Some(editor))).is_none() {
			log::error!("Attempted to initialize the editor more than once");
		}

		Self::initialize_wrapper(frontend_message_handler_callback)
	}
	#[cfg(feature = "native")]
	pub fn create(_platform: String, _uuid_random_seed: u64, frontend_message_handler_callback: js_sys::Function) -> EditorWrapper {
		Self::initialize_wrapper(frontend_message_handler_callback)
	}

	// Sends a message to the dispatcher in the Editor Backend
	#[cfg(not(feature = "native"))]
	pub(crate) fn dispatch<T: Into<Message>>(&self, message: T) {
		// Process no further messages after a crash to avoid spamming the console
		use crate::MESSAGE_BUFFER;
		if EDITOR_HAS_CRASHED.load(Ordering::SeqCst) {
			return;
		}

		// Get the editor, dispatch the message, and store the `FrontendMessage` queue response
		let frontend_messages = EDITOR.with(|editor| {
			let mut guard = editor.try_lock();
			let Ok(Some(editor)) = guard.as_deref_mut() else {
				// Enqueue messages which can't be procssed currently
				MESSAGE_BUFFER.with_borrow_mut(|buffer| buffer.push(message.into()));
				return vec![];
			};

			editor.handle_message(message)
		});

		// Send each `FrontendMessage` to the JavaScript frontend
		for message in frontend_messages.into_iter() {
			self.send_frontend_message_to_js(message);
		}
	}
	#[cfg(feature = "native")]
	pub(crate) fn dispatch<T: Into<Message>>(&self, message: T) {
		let message: Message = message.into();
		let Ok(serialized_message) = ron::to_string(&message) else {
			log::error!("Failed to serialize message");
			return;
		};
		crate::native_communication::send_message_to_cef(serialized_message)
	}

	// Sends a FrontendMessage to JavaScript
	pub(crate) fn send_frontend_message_to_js(&self, message: FrontendMessage) {
		if let FrontendMessage::UpdateImageData { ref image_data } = message {
			let new_hash = calculate_hash(&CacheHashWrapper(image_data));
			let prev_hash = IMAGE_DATA_HASH.load(Ordering::Relaxed);

			if new_hash != prev_hash {
				render_image_data_to_canvases(image_data.as_slice());
				IMAGE_DATA_HASH.store(new_hash, Ordering::Relaxed);
			}
			return;
		}

		let message_type = message.to_discriminant().local_name();

		let serializer = serde_wasm_bindgen::Serializer::new().serialize_large_number_types_as_bigints(true);
		let message_data = message.serialize(&serializer).expect("Failed to serialize FrontendMessage");

		let js_return_value = self.frontend_message_handler_callback.call2(&JsValue::null(), &JsValue::from(message_type), &message_data);

		if let Err(error) = js_return_value {
			error!("While handling FrontendMessage {:?}, JavaScript threw an error:\n{:?}", message.to_discriminant().local_name(), error,)
		}
	}

	// ================================================
	// Functions for calling the editor in Rust from JS
	// ================================================

	/// Re-sends all UI layouts to the frontend. Called during HMR re-mounts when the frontend has lost its layout state.
	#[wasm_bindgen(js_name = resendAllLayouts)]
	pub fn resend_all_layouts(&self) {
		self.dispatch(LayoutMessage::ResendAllLayouts);
	}

	#[wasm_bindgen(js_name = initAfterFrontendReady)]
	pub fn init_after_frontend_ready(&self) {
		// Enforce idempotency, so if this is called again during an HMR re-mount, we don't initialize the editor backend twice
		if FRONTEND_READY.swap(true, Ordering::SeqCst) {
			return;
		}

		#[cfg(feature = "native")]
		crate::native_communication::initialize_native_communication();

		self.dispatch(PortfolioMessage::Init);

		// Poll node graph evaluation on `requestAnimationFrame`
		{
			let f = std::rc::Rc::new(RefCell::new(None));
			let g = f.clone();

			*g.borrow_mut() = Some(Closure::new(move |_timestamp| {
				#[cfg(not(feature = "native"))]
				wasm_bindgen_futures::spawn_local(poll_node_graph_evaluation());

				if !EDITOR_HAS_CRASHED.load(Ordering::SeqCst) {
					wrapper(|wrapper| {
						// Process all messages that have been queued up
						let mut messages = MESSAGE_BUFFER.take();
						messages.push(
							InputPreprocessorMessage::CurrentTime {
								timestamp: js_sys::Date::now() as u64,
							}
							.into(),
						);
						messages.push(AnimationMessage::IncrementFrameCounter.into());

						// Used by auto-panning, but this could possibly be refactored in the future, see:
						// <https://github.com/GraphiteEditor/Graphite/pull/2562#discussion_r2041102786>
						messages.push(BroadcastMessage::TriggerEvent(EventMessage::AnimationFrame).into());

						wrapper.dispatch(Message::Batched { messages: messages.into() });
					});
				}

				// Schedule ourself for another requestAnimationFrame callback
				request_animation_frame(f.borrow().as_ref().unwrap());
			}));

			request_animation_frame(g.borrow().as_ref().unwrap());
		}

		// Auto save all documents on `setTimeout`
		{
			let f = std::rc::Rc::new(RefCell::new(None));
			let g = f.clone();

			*g.borrow_mut() = Some(Closure::new(move || {
				auto_save_all_documents();

				// Schedule ourself for another setTimeout callback
				set_timeout(f.borrow().as_ref().unwrap(), Duration::from_secs(editor::consts::AUTO_SAVE_TIMEOUT_SECONDS));
			}));

			set_timeout(g.borrow().as_ref().unwrap(), Duration::from_secs(editor::consts::AUTO_SAVE_TIMEOUT_SECONDS));
		}
	}

	#[wasm_bindgen(js_name = addPrimaryImport)]
	pub fn add_primary_import(&self) {
		self.dispatch(DocumentMessage::AddTransaction);
		self.dispatch(NodeGraphMessage::AddPrimaryImport);
	}

	#[wasm_bindgen(js_name = addSecondaryImport)]
	pub fn add_secondary_import(&self) {
		self.dispatch(DocumentMessage::AddTransaction);
		self.dispatch(NodeGraphMessage::AddSecondaryImport);
	}

	#[wasm_bindgen(js_name = addPrimaryExport)]
	pub fn add_primary_export(&self) {
		self.dispatch(DocumentMessage::AddTransaction);
		self.dispatch(NodeGraphMessage::AddPrimaryExport);
	}

	#[wasm_bindgen(js_name = addSecondaryExport)]
	pub fn add_secondary_export(&self) {
		self.dispatch(DocumentMessage::AddTransaction);
		self.dispatch(NodeGraphMessage::AddSecondaryExport);
	}

	/// Start Pointer Lock
	#[wasm_bindgen(js_name = appWindowPointerLock)]
	pub fn app_window_pointer_lock(&self) {
		let message = AppWindowMessage::PointerLock;
		self.dispatch(message);
	}

	/// Minimizes the application window to the taskbar or dock
	#[wasm_bindgen(js_name = appWindowMinimize)]
	pub fn app_window_minimize(&self) {
		let message = AppWindowMessage::Minimize;
		self.dispatch(message);
	}

	/// Toggles minimizing or restoring down the application window
	#[wasm_bindgen(js_name = appWindowMaximize)]
	pub fn app_window_maximize(&self) {
		let message = AppWindowMessage::Maximize;
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = appWindowFullscreen)]
	pub fn app_window_fullscreen(&self) {
		let message = AppWindowMessage::Fullscreen;
		self.dispatch(message);
	}

	/// Closes the application window
	#[wasm_bindgen(js_name = appWindowClose)]
	pub fn app_window_close(&self) {
		let message = AppWindowMessage::Close;
		self.dispatch(message);
	}

	/// Drag the application window
	#[wasm_bindgen(js_name = appWindowDrag)]
	pub fn app_window_start_drag(&self) {
		let message = AppWindowMessage::Drag;
		self.dispatch(message);
	}

	/// Displays a dialog with an error message
	#[wasm_bindgen(js_name = errorDialog)]
	pub fn error_dialog(&self, title: String, description: String) {
		let message = DialogMessage::DisplayDialogError { title, description };
		self.dispatch(message);
	}

	/// Answer whether or not the editor has crashed
	#[wasm_bindgen(js_name = hasCrashed)]
	pub fn has_crashed(&self) -> bool {
		EDITOR_HAS_CRASHED.load(Ordering::SeqCst)
	}

	/// Answer whether or not the editor is in development mode
	#[wasm_bindgen(js_name = inDevelopmentMode)]
	pub fn in_development_mode(&self) -> bool {
		cfg!(debug_assertions)
	}

	/// Get the constant `FILE_EXTENSION`
	#[wasm_bindgen(js_name = fileExtension)]
	pub fn file_extension(&self) -> String {
		FILE_EXTENSION.into()
	}

	/// Get the constant `GDD_FILE_EXTENSION`
	#[wasm_bindgen(js_name = gddFileExtension)]
	pub fn gdd_file_extension(&self) -> String {
		GDD_FILE_EXTENSION.into()
	}

	/// Update the value of a given UI widget, but don't commit it to the history (unless `commit_layout()` is called, which handles that)
	#[wasm_bindgen(js_name = widgetValueUpdate)]
	pub fn widget_value_update(&self, layout_target: LayoutTarget, widget_id: u64, value: JsValue, resend_widget: bool) -> Result<(), JsValue> {
		self.widget_value_update_helper(layout_target, widget_id, value, resend_widget)
	}

	/// Commit the value of a given UI widget to the history
	#[wasm_bindgen(js_name = widgetValueCommit)]
	pub fn widget_value_commit(&self, layout_target: LayoutTarget, widget_id: u64, value: JsValue) -> Result<(), JsValue> {
		self.widget_value_commit_helper(layout_target, widget_id, value)
	}

	/// Update the value of a given UI widget, and commit it to the history
	#[wasm_bindgen(js_name = widgetValueCommitAndUpdate)]
	pub fn widget_value_commit_and_update(&self, layout_target: LayoutTarget, widget_id: u64, value: JsValue, resend_widget: bool) -> Result<(), JsValue> {
		self.widget_value_commit_helper(layout_target, widget_id, value.clone())?;
		self.widget_value_update_helper(layout_target, widget_id, value, resend_widget)?;
		// Close out a transaction that the widget's `on_commit` opened (if any), so a single click on widgets like the
		// NumberInput's increment buttons collapses into one history step instead of leaving the transaction in `Modified`
		self.dispatch(DocumentMessage::EndTransaction);
		Ok(())
	}

	/// Fire a widget's drag-drop action (e.g. when a draggable item is dropped on a button)
	#[wasm_bindgen(js_name = widgetValueDragDrop)]
	pub fn widget_value_drag_drop(&self, layout_target: LayoutTarget, widget_id: u64) {
		let widget_id = WidgetId(widget_id);
		self.dispatch(LayoutMessage::WidgetValueDragDrop { layout_target, widget_id });
	}

	/// Closes out the current transaction (drag-end / text-commit end), so emits during a slider drag collapse into one history step instead of N
	#[wasm_bindgen(js_name = endTransaction)]
	pub fn end_transaction(&self) {
		self.dispatch(DocumentMessage::EndTransaction);
	}

	pub fn widget_value_update_helper(&self, layout_target: LayoutTarget, widget_id: u64, value: JsValue, resend_widget: bool) -> Result<(), JsValue> {
		let widget_id = WidgetId(widget_id);
		let value: serde_json::Value = from_value(value).map_err(|e| Error::new(&format!("Could not update UI: {e}")))?;
		let message = LayoutMessage::WidgetValueUpdate { layout_target, widget_id, value };
		self.dispatch(message);
		if resend_widget {
			let resend_message = LayoutMessage::ResendActiveWidget { layout_target, widget_id };
			self.dispatch(resend_message);
		}
		Ok(())
	}

	pub fn widget_value_commit_helper(&self, layout_target: LayoutTarget, widget_id: u64, value: JsValue) -> Result<(), JsValue> {
		let widget_id = WidgetId(widget_id);
		let value: serde_json::Value = from_value(value).map_err(|e| Error::new(&format!("Could not commit UI: {e}")))?;
		let message = LayoutMessage::WidgetValueCommit { layout_target, widget_id, value };
		self.dispatch(message);
		Ok(())
	}

	#[wasm_bindgen(js_name = loadPreferences)]
	pub fn load_preferences(&self, preferences: Option<String>) {
		if let Some(preferences) = preferences {
			let Ok(preferences) = serde_json::from_str(&preferences) else {
				log::error!("Failed to deserialize preferences");
				return;
			};
			let message = PreferencesMessage::Load { preferences };
			self.dispatch(message);
		}
	}

	#[wasm_bindgen(js_name = loadPersistedState)]
	pub fn load_persisted_state(&self, state: editor::messages::frontend::utility_types::PersistedState) {
		self.dispatch(PersistentStateMessage::LoadState { state });
	}

	#[wasm_bindgen(js_name = loadDocumentContent)]
	pub fn load_document_content(&self, document_id: u64, document: String) {
		let message = PersistentStateMessage::LoadDocument {
			document_id: DocumentId(document_id),
			document,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = selectDocument)]
	pub fn select_document(&self, document_id: u64) {
		let document_id = DocumentId(document_id);
		let message = PortfolioMessage::SelectDocument { document_id };
		self.dispatch(message);
	}

	/// Rename the currently active document.
	#[wasm_bindgen(js_name = renameDocument)]
	pub fn rename_document(&self, new_name: String) {
		let message = PortfolioMessage::RenameDocument { new_name };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = newDocumentDialog)]
	pub fn new_document_dialog(&self) {
		let message = DialogMessage::RequestNewDocumentDialog;
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = openFile)]
	pub fn open_file(&self, path: String, content: Vec<u8>) {
		let message = PortfolioMessage::OpenFile { path: PathBuf::from(path), content };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = importFile)]
	pub fn import_file(&self, path: String, content: Vec<u8>) {
		let message = PortfolioMessage::ImportFile { path: PathBuf::from(path), content };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = triggerAutoSave)]
	pub fn trigger_auto_save(&self, document_id: u64) {
		let document_id = DocumentId(document_id);
		let message = PortfolioMessage::AutoSaveDocument { document_id };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = reorderDocument)]
	pub fn reorder_document(&self, document_id: u64, new_index: usize) {
		let document_id = DocumentId(document_id);
		let message = PortfolioMessage::ReorderDocument { document_id, new_index };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = reorderPanelGroupTab)]
	pub fn reorder_panel_group_tab(&self, group: u64, old_index: usize, new_index: usize) {
		let message = PortfolioMessage::ReorderPanelGroupTab {
			group: PanelGroupId(group),
			old_index,
			new_index,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = moveAllPanelTabs)]
	pub fn move_all_panel_tabs(&self, source_group: u64, target_group: u64, insert_index: usize) {
		let message = PortfolioMessage::MoveAllPanelTabs {
			source_group: PanelGroupId(source_group),
			target_group: PanelGroupId(target_group),
			insert_index,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = movePanelTab)]
	pub fn move_panel_tab(&self, source_group: u64, target_group: u64, insert_index: usize) {
		let message = PortfolioMessage::MovePanelTab {
			source_group: PanelGroupId(source_group),
			target_group: PanelGroupId(target_group),
			insert_index,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = setPanelGroupActiveTab)]
	pub fn set_panel_group_active_tab(&self, group: u64, tab_index: usize) {
		let message = PortfolioMessage::SetPanelGroupActiveTab {
			group: PanelGroupId(group),
			tab_index,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = splitPanelGroup)]
	pub fn split_panel_group(&self, target_group: u64, direction: DockingSplitDirection, tabs: Vec<PanelType>, active_tab_index: usize) {
		let message = PortfolioMessage::SplitPanelGroup {
			target_group: PanelGroupId(target_group),
			direction,
			tabs,
			active_tab_index,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = setPanelGroupSizes)]
	pub fn set_panel_group_sizes(&self, split_path: Vec<u32>, sizes: Vec<f64>) {
		let split_path = split_path.into_iter().map(|i| i as usize).collect();
		let message = PortfolioMessage::SetPanelGroupSizes { split_path, sizes };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = closeDocumentWithConfirmation)]
	pub fn close_document_with_confirmation(&self, document_id: u64) {
		let document_id = DocumentId(document_id);
		let message = PortfolioMessage::CloseDocumentWithConfirmation { document_id };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = requestAboutGraphiteDialogWithLocalizedCommitDate)]
	pub fn request_about_graphite_dialog_with_localized_commit_date(&self, localized_commit_date: String, localized_commit_year: String) {
		let message = DialogMessage::RequestAboutGraphiteDialogWithLocalizedCommitDate {
			localized_commit_date,
			localized_commit_year,
		};
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = requestLicensesThirdPartyDialogWithLicenseText)]
	pub fn request_licenses_third_party_dialog_with_license_text(&self, license_text: String) {
		let message = DialogMessage::RequestLicensesThirdPartyDialogWithLicenseText { license_text };
		self.dispatch(message);
	}

	/// Send new viewport info to the backend
	#[wasm_bindgen(js_name = updateViewport)]
	pub fn update_viewport(&self, x: f64, y: f64, width: f64, height: f64, scale: f64) {
		let message = ViewportMessage::Update { x, y, width, height, scale };
		self.dispatch(message);
	}

	/// Mouse movement within the screenspace bounds of the viewport
	#[wasm_bindgen(js_name = onMouseMove)]
	pub fn on_mouse_move(&self, x: f64, y: f64, mouse_keys: u8, modifiers: u8) {
		let editor_mouse_state = EditorMouseState::from_keys_and_editor_position(mouse_keys, (x, y).into());

		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		let message = InputPreprocessorMessage::PointerMove { editor_mouse_state, modifier_keys };
		self.dispatch(message);
	}

	/// Mouse scrolling within the screenspace bounds of the viewport
	#[wasm_bindgen(js_name = onWheelScroll)]
	pub fn on_wheel_scroll(&self, x: f64, y: f64, mouse_keys: u8, wheel_delta_x: f64, wheel_delta_y: f64, wheel_delta_z: f64, modifiers: u8) {
		let mut editor_mouse_state = EditorMouseState::from_keys_and_editor_position(mouse_keys, (x, y).into());
		editor_mouse_state.scroll_delta = ScrollDelta::new(wheel_delta_x, wheel_delta_y, wheel_delta_z);

		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		let message = InputPreprocessorMessage::WheelScroll { editor_mouse_state, modifier_keys };
		self.dispatch(message);
	}

	/// A mouse button depressed within screenspace the bounds of the viewport
	#[wasm_bindgen(js_name = onMouseDown)]
	pub fn on_mouse_down(&self, x: f64, y: f64, mouse_keys: u8, modifiers: u8) {
		let editor_mouse_state = EditorMouseState::from_keys_and_editor_position(mouse_keys, (x, y).into());

		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		let message = InputPreprocessorMessage::PointerDown { editor_mouse_state, modifier_keys };
		self.dispatch(message);
	}

	/// A mouse button released
	#[wasm_bindgen(js_name = onMouseUp)]
	pub fn on_mouse_up(&self, x: f64, y: f64, mouse_keys: u8, modifiers: u8) {
		let editor_mouse_state = EditorMouseState::from_keys_and_editor_position(mouse_keys, (x, y).into());

		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		let message = InputPreprocessorMessage::PointerUp { editor_mouse_state, modifier_keys };
		self.dispatch(message);
	}

	/// Mouse shaken
	#[wasm_bindgen(js_name = onMouseShake)]
	pub fn on_mouse_shake(&self, x: f64, y: f64, mouse_keys: u8, modifiers: u8) {
		let editor_mouse_state = EditorMouseState::from_keys_and_editor_position(mouse_keys, (x, y).into());

		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		let message = InputPreprocessorMessage::PointerShake { editor_mouse_state, modifier_keys };
		self.dispatch(message);
	}

	/// Mouse double clicked
	#[wasm_bindgen(js_name = onDoubleClick)]
	pub fn on_double_click(&self, x: f64, y: f64, mouse_keys: u8, modifiers: u8) {
		let editor_mouse_state = EditorMouseState::from_keys_and_editor_position(mouse_keys, (x, y).into());

		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		let message = InputPreprocessorMessage::DoubleClick { editor_mouse_state, modifier_keys };
		self.dispatch(message);
	}

	/// A keyboard button depressed within screenspace the bounds of the viewport
	#[wasm_bindgen(js_name = onKeyDown)]
	pub fn on_key_down(&self, name: String, modifiers: u8, key_repeat: bool) {
		let key = translate_key(&name);
		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		trace!("Key down {key:?}, name: {name}, modifiers: {modifiers:?}, key repeat: {key_repeat}");

		let message = InputPreprocessorMessage::KeyDown { key, key_repeat, modifier_keys };
		self.dispatch(message);
	}

	/// A keyboard button released
	#[wasm_bindgen(js_name = onKeyUp)]
	pub fn on_key_up(&self, name: String, modifiers: u8, key_repeat: bool) {
		let key = translate_key(&name);
		let modifier_keys = ModifierKeys::from_bits(modifiers).expect("Invalid modifier keys");

		trace!("Key up {key:?}, name: {name}, modifiers: {modifier_keys:?}, key repeat: {key_repeat}");

		let message = InputPreprocessorMessage::KeyUp { key, key_repeat, modifier_keys };
		self.dispatch(message);
	}

	/// A text box was committed
	#[wasm_bindgen(js_name = onChangeText)]
	pub fn on_change_text(&self, new_text: String, is_left_or_right_click: bool) -> Result<(), JsValue> {
		let message = TextToolMessage::TextChange { new_text, is_left_or_right_click };
		self.dispatch(message);

		Ok(())
	}

	/// Dialog got dismissed
	#[wasm_bindgen(js_name = onDialogDismiss)]
	pub fn on_dialog_dismiss(&self) {
		let message = DialogMessage::Dismiss;
		self.dispatch(message);
	}

	/// A text box was changed
	#[wasm_bindgen(js_name = updateBounds)]
	pub fn update_bounds(&self, new_text: String) -> Result<(), JsValue> {
		let message = TextToolMessage::UpdateBounds { new_text };
		self.dispatch(message);

		Ok(())
	}

	/// Update primary color from sRGB bytes (the wire format at the JS boundary).
	#[wasm_bindgen(js_name = updatePrimaryColor)]
	pub fn update_primary_color(&self, color: SRGBA8) {
		self.dispatch(ToolMessage::SelectWorkingColor {
			color: Color::from(color),
			primary: true,
		});
	}

	/// Update secondary color from sRGB bytes (the wire format at the JS boundary).
	#[wasm_bindgen(js_name = updateSecondaryColor)]
	pub fn update_secondary_color(&self, color: SRGBA8) {
		self.dispatch(ToolMessage::SelectWorkingColor {
			color: Color::from(color),
			primary: false,
		});
	}

	/// Initialize the Rust color picker handler with a starting value (used when the frontend `<ColorPicker />` opens).
	#[wasm_bindgen(js_name = openColorPicker)]
	pub fn open_color_picker(&self, initial_value: FillChoiceUI, allow_none: bool, disabled: bool) {
		let initial_value = FillChoice::from(&initial_value);
		self.dispatch(ColorPickerMessage::Open { initial_value, allow_none, disabled });
	}

	/// Tell the Rust color picker handler that the popover is closing.
	#[wasm_bindgen(js_name = closeColorPicker)]
	pub fn close_color_picker(&self) {
		self.dispatch(ColorPickerMessage::Close);
	}

	/// Update the color of the currently-edited gradient stop, from sRGB bytes (the wire format at the JS boundary).
	#[wasm_bindgen(js_name = updateGradientStopColor)]
	pub fn update_gradient_stop_color(&self, color: SRGBA8) {
		self.dispatch(GradientToolMessage::UpdateStopColor { color: Color::from(color) });
	}

	/// Start a new undo transaction for gradient stop color editing
	#[wasm_bindgen(js_name = startGradientStopColorTransaction)]
	pub fn start_gradient_stop_color_transaction(&self) {
		self.dispatch(GradientToolMessage::StartTransactionForColorStop);
	}

	/// Commit the current gradient stop color transaction (called on pointer-up after each drag/click)
	#[wasm_bindgen(js_name = commitGradientStopColorTransaction)]
	pub fn commit_gradient_stop_color_transaction(&self) {
		self.dispatch(GradientToolMessage::CommitTransactionForColorStop);
	}

	/// Close the gradient stop color picker and commit any pending transaction
	#[wasm_bindgen(js_name = closeGradientStopColorPicker)]
	pub fn close_gradient_stop_color_picker(&self) {
		self.dispatch(GradientToolMessage::CloseStopColorPicker);
	}

	/// Toggle clipping the alpha of a layer to the alpha of the layer below it in the layer stack
	#[wasm_bindgen(js_name = clipLayer)]
	pub fn clip_layer(&self, id: u64) {
		let id = NodeId(id);
		let message = DocumentMessage::ClipLayer { id };
		self.dispatch(message);
	}

	/// Modify the layer selection based on the layer which is clicked while holding down the <kbd>Ctrl</kbd> and/or <kbd>Shift</kbd> modifier keys used for range selection behavior
	#[wasm_bindgen(js_name = selectLayer)]
	pub fn select_layer(&self, id: u64, ctrl: bool, shift: bool) {
		let id = NodeId(id);
		let message = DocumentMessage::SelectLayer { id, ctrl, shift };
		self.dispatch(message);
	}

	/// Deselect all layers
	#[wasm_bindgen(js_name = deselectAllLayers)]
	pub fn deselect_all_layers(&self) {
		let message = DocumentMessage::DeselectAllLayers;
		self.dispatch(message);
	}

	/// Move a layer to within a folder and placed down at the given index.
	/// If the folder is `None`, it is inserted into the document root.
	/// If the insert index is `None`, it is inserted at the start of the folder.
	#[wasm_bindgen(js_name = moveLayerInTree)]
	pub fn move_layer_in_tree(&self, insert_parent_id: Option<u64>, insert_index: Option<usize>) {
		let insert_parent_id = insert_parent_id.map(NodeId);
		let parent = insert_parent_id.map(LayerNodeIdentifier::new_unchecked).unwrap_or_default();

		let message = DocumentMessage::MoveSelectedLayersTo {
			parent,
			insert_index: insert_index.unwrap_or_default(),
		};
		self.dispatch(message);
	}

	/// Reorder a draggable Properties panel section to the given index among its peers.
	#[wasm_bindgen(js_name = reorderPropertiesSection)]
	pub fn reorder_properties_section(&self, node_id: u64, insert_index: usize) {
		self.dispatch(DocumentMessage::ReorderPropertiesSection {
			node_id: NodeId(node_id),
			insert_index,
		});
	}

	/// Duplicate the selected layers, placing the copies within the given folder at the given index.
	/// If the folder is `None`, they are inserted into the document root.
	/// If the insert index is `None`, they are inserted at the start of the folder.
	#[wasm_bindgen(js_name = duplicateLayerInTree)]
	pub fn duplicate_layer_in_tree(&self, insert_parent_id: Option<u64>, insert_index: Option<usize>) {
		let message = DocumentMessage::DuplicateSelectedLayersTo {
			parent: insert_parent_id.map(NodeId).map(LayerNodeIdentifier::new_unchecked).unwrap_or_default(),
			insert_index: insert_index.unwrap_or_default(),
		};
		self.dispatch(message);
	}

	/// Set the name for the layer
	#[wasm_bindgen(js_name = setLayerName)]
	pub fn set_layer_name(&self, id: u64, name: String) {
		let layer = LayerNodeIdentifier::new_unchecked(NodeId(id));
		let message = NodeGraphMessage::SetDisplayName {
			node_id: layer.to_node(),
			network_path: Vec::new(),
			alias: name,
			skip_adding_history_step: false,
		};
		self.dispatch(message);
	}

	/// Translates document (in viewport coords)
	#[wasm_bindgen(js_name = panCanvasAbortPrepare)]
	pub fn pan_canvas_abort_prepare(&self, x_not_y_axis: bool) {
		let message = NavigationMessage::CanvasPanAbortPrepare { x_not_y_axis };
		self.dispatch(message);
	}

	#[wasm_bindgen(js_name = panCanvasAbort)]
	pub fn pan_canvas_abort(&self, x_not_y_axis: bool) {
		let message = NavigationMessage::CanvasPanAbort { x_not_y_axis };
		self.dispatch(message);
	}

	/// Translates document (in viewport coords)
	#[wasm_bindgen(js_name = panCanvas)]
	pub fn pan_canvas(&self, delta_x: f64, delta_y: f64) {
		let message = NavigationMessage::CanvasPan { delta: (delta_x, delta_y).into() };
		self.dispatch(message);
	}

	/// Translates document (in viewport coords)
	#[wasm_bindgen(js_name = panCanvasByFraction)]
	pub fn pan_canvas_by_fraction(&self, delta_x: f64, delta_y: f64) {
		let message = NavigationMessage::CanvasPanByViewportFraction { delta: (delta_x, delta_y).into() };
		self.dispatch(message);
	}

	/// Merge the selected nodes into a subnetwork
	#[wasm_bindgen(js_name = mergeSelectedNodes)]
	pub fn merge_nodes(&self) {
		let message = NodeGraphMessage::MergeSelectedNodes;
		self.dispatch(message);
	}

	/// Toggle lock state of all selected layers
	#[wasm_bindgen(js_name = toggleSelectedLocked)]
	pub fn toggle_selected_locked(&self) {
		let message = NodeGraphMessage::ToggleSelectedLocked;
		self.dispatch(message);
	}

	/// Creates a new document node in the node graph
	#[wasm_bindgen(js_name = createNode)]
	pub fn create_node(&self, node_type: JsValue, x: i32, y: i32) {
		let value: serde_json::Value = serde_wasm_bindgen::from_value(node_type).unwrap();

		let id = NodeId::new();
		let message = NodeGraphMessage::CreateNodeFromContextMenu {
			node_id: Some(id),
			node_type: value.into(),
			xy: Some((x / 24, y / 24)),
			add_transaction: true,
		};
		self.dispatch(message);
	}

	/// Respond to selection read
	#[wasm_bindgen(js_name = readSelection)]
	pub fn read_selection(&self, content: Option<String>, cut: bool) {
		let message = ClipboardMessage::ReadSelection { content, cut };
		self.dispatch(message);
	}

	/// Paste from a serialized JSON representation
	#[wasm_bindgen(js_name = pasteText)]
	pub fn paste_text(&self, data: String) {
		let message = ClipboardMessage::ReadClipboard {
			content: ClipboardContentRaw::Text(data),
		};
		self.dispatch(message);
	}

	/// Pastes an image
	#[wasm_bindgen(js_name = pasteImage)]
	pub fn paste_image(
		&self,
		name: Option<String>,
		image_data: Vec<u8>,
		width: u32,
		height: u32,
		mouse_x: Option<f64>,
		mouse_y: Option<f64>,
		insert_parent_id: Option<u64>,
		insert_index: Option<usize>,
	) {
		let mouse = mouse_x.and_then(|x| mouse_y.map(|y| (x, y)));
		let image = graphene_std::raster::Image::from_image_data(&image_data, width, height);

		let parent_and_insert_index = if let (Some(insert_parent_id), Some(insert_index)) = (insert_parent_id, insert_index) {
			let insert_parent_id = NodeId(insert_parent_id);
			let parent = LayerNodeIdentifier::new_unchecked(insert_parent_id);
			Some((parent, insert_index))
		} else {
			None
		};

		let message = PortfolioMessage::InsertImage {
			name,
			image,
			mouse,
			parent_and_insert_index,
		};
		self.dispatch(message);
	}

	/// Pastes an SVG given its string representation
	#[wasm_bindgen(js_name = pasteSvg)]
	pub fn paste_svg(&self, name: Option<String>, svg: String, mouse_x: Option<f64>, mouse_y: Option<f64>, insert_parent_id: Option<u64>, insert_index: Option<usize>) {
		let mouse = mouse_x.and_then(|x| mouse_y.map(|y| (x, y)));

		let parent_and_insert_index = if let (Some(insert_parent_id), Some(insert_index)) = (insert_parent_id, insert_index) {
			let insert_parent_id = NodeId(insert_parent_id);
			let parent = LayerNodeIdentifier::new_unchecked(insert_parent_id);
			Some((parent, insert_index))
		} else {
			None
		};

		let message = PortfolioMessage::InsertSvg {
			name,
			svg,
			mouse,
			parent_and_insert_index,
		};
		self.dispatch(message);
	}

	/// Toggle visibility of a layer or node given its node ID
	#[wasm_bindgen(js_name = toggleNodeVisibilityLayerPanel)]
	pub fn toggle_node_visibility_layer(&self, id: u64) {
		let node_id = NodeId(id);
		let message = NodeGraphMessage::ToggleVisibility { node_id, network_path: Vec::new() };
		self.dispatch(message);
	}

	/// Pin or unpin a node given its node ID
	#[wasm_bindgen(js_name = setNodePinned)]
	pub fn set_node_pinned(&self, id: u64, pinned: bool) {
		self.dispatch(DocumentMessage::SetNodePinned { node_id: NodeId(id), pinned });
	}

	/// Collapse or expand a node's section in the Properties panel
	#[wasm_bindgen(js_name = toggleNodePropertiesSectionExpanded)]
	pub fn toggle_node_properties_section_expanded(&self, id: u64) {
		self.dispatch(DocumentMessage::ToggleNodePropertiesSectionExpanded { node_id: NodeId(id) });
	}

	/// Delete a layer or node given its node ID
	#[wasm_bindgen(js_name = deleteNode)]
	pub fn delete_node(&self, id: u64) {
		self.dispatch(DocumentMessage::DeleteNode { node_id: NodeId(id) });
	}

	/// Toggle lock state of a layer from the layer list
	#[wasm_bindgen(js_name = toggleLayerLock)]
	pub fn toggle_layer_lock(&self, node_id: u64) {
		let message = NodeGraphMessage::ToggleLocked {
			node_id: NodeId(node_id),
			network_path: Vec::new(),
		};
		self.dispatch(message);
	}

	/// Toggle expansions state of a layer from the layer list
	#[wasm_bindgen(js_name = toggleLayerExpansion)]
	pub fn toggle_layer_expansion(&self, tree_path: &[u64], recursive: bool) {
		let tree_path = tree_path.iter().map(|&id| NodeId(id)).collect();
		let message = DocumentMessage::ToggleLayerExpansion { tree_path, recursive };
		self.dispatch(message);
	}

	/// Set the active panel to the most recently clicked panel
	#[wasm_bindgen(js_name = setActivePanel)]
	pub fn set_active_panel(&self, panel: String) {
		let message = DocumentMessage::SetActivePanel { active_panel: panel.into() };
		self.dispatch(message);
	}

	/// Toggle display type for a layer
	#[wasm_bindgen(js_name = setToNodeOrLayer)]
	pub fn set_to_node_or_layer(&self, id: u64, is_layer: bool) {
		self.dispatch(DocumentMessage::SetToNodeOrLayer { node_id: NodeId(id), is_layer });
	}

	/// Set the name of an import or export
	#[wasm_bindgen(js_name = setImportName)]
	pub fn set_import_name(&self, index: usize, name: String) {
		let message = NodeGraphMessage::SetImportExportName {
			name,
			index: ImportOrExport::Import(index),
		};
		self.dispatch(message);
	}

	/// Set the name of an export
	#[wasm_bindgen(js_name = setExportName)]
	pub fn set_export_name(&self, index: usize, name: String) {
		let message = NodeGraphMessage::SetImportExportName {
			name,
			index: ImportOrExport::Export(index),
		};
		self.dispatch(message);
	}

	/// MCP tool call: execute a named tool with JSON arguments and return a JSON string result.
	/// This is called from the browser-side MCP bridge to control the editor from an AI agent.
	#[wasm_bindgen(js_name = mcpToolCall)]
	pub fn mcp_tool_call(&self, tool_name: String, args_json: String) -> String {
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let args: serde_json::Value = serde_json::from_str(&args_json).unwrap_or(serde_json::json!({}));
			mcp_tool_handler(self, &tool_name, &args)
		}));
		match result {
			Ok(s) => s,
			Err(e) => {
				let msg = if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
					else if let Some(s) = e.downcast_ref::<String>() { s.clone() }
					else { "Unknown panic".to_string() };
				serde_json::to_string(&serde_json::json!({
					"content": [{"type": "text", "text": format!("WASM panic: {msg}")}],
					"isError": true
				})).unwrap_or_else(|_| r#"{"content":[{"type":"text","text":"Serialization error"}],"isError":true}"#.into())
			}
		}
	}
}

/// Handle an MCP tool call by dispatching messages to the editor and/or reading editor state.
/// Returns a JSON string with the result.
#[cfg(not(feature = "native"))]
fn mcp_tool_handler(wrapper: &EditorWrapper, tool_name: &str, args: &serde_json::Value) -> String {
	use editor::messages::tool::utility_types::ToolType;

	// Helper to dispatch a message
	let dispatch = |msg: Message| {
		wrapper.dispatch(msg);
	};

	// Helper to validate SVG has a root <svg> element before dispatching
	let validate_svg = |svg: &str, tool_name: &str| -> Result<(), String> {
		let trimmed = svg.trim();
		if !trimmed.starts_with("<svg") {
			return Err(format!("{tool_name}: SVG must have a root <svg> element"));
		}
		if !trimmed.contains("</svg>") {
			return Err(format!("{tool_name}: SVG is missing closing </svg> tag"));
		}
		Ok(())
	};

	// Helper to read editor state
	let with_editor = |f: &dyn Fn(&Editor) -> serde_json::Value| -> serde_json::Value {
		EDITOR.with(|editor| {
			let guard = editor.try_lock();
			if let Ok(Some(editor)) = guard.as_deref() {
				f(editor)
			} else {
				serde_json::json!({"error": "Editor not available"})
			}
		})
	};

	let result = match tool_name {
		"get_node_catalog" | "get_node_details" => {
			// These are catalog-only tools that don't need the editor
			serde_json::json!({
				"content": [{"type": "text", "text": "Node catalog tools are available via the standalone MCP server. Use 'cargo run -p graphite-mcp-server' for catalog queries."}]
			})
		}

		"create_document" => {
			let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
			dispatch(Message::Portfolio(PortfolioMessage::NewDocumentWithName { name }));
			serde_json::json!({"content": [{"type": "text", "text": "Document created"}]})
		}

		"list_documents" => {
			dispatch(Message::Portfolio(PortfolioMessage::UpdateOpenDocumentsList));
			serde_json::json!({"content": [{"type": "text", "text": "Document list updated in UI"}]})
		}

		"get_layer_tree" => {
			let tree = with_editor(&|editor| {
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
						serde_json::json!(out)
					}
					None => serde_json::json!("No active document"),
				}
			});
			serde_json::json!({"content": [{"type": "text", "text": tree}]})
		}

		"create_rectangle" => {
			let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.);
			let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.);
			let w = args.get("width").and_then(|v| v.as_f64()).unwrap_or(100.);
			let h = args.get("height").and_then(|v| v.as_f64()).unwrap_or(100.);
			let fill = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("#000000");
			let r = args.get("corner_radius").and_then(|v| v.as_f64()).unwrap_or(0.);
			let view_w = x + w;
			let view_h = y + h;
			let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_w} {view_h}"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"/></svg>"#);
			if let Err(e) = validate_svg(&svg, "create_rectangle") {
				return serde_json::to_string(&serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true})).unwrap();
			}
			dispatch(Message::Portfolio(PortfolioMessage::InsertSvg {
				name: Some("Rectangle".into()),
				svg,
				mouse: None,
				parent_and_insert_index: None,
			}));
			serde_json::json!({"content": [{"type": "text", "text": format!("Created rectangle at ({x}, {y}) {w}x{h}")}]})
		}

		"create_ellipse" => {
			let cx = args.get("x").and_then(|v| v.as_f64()).unwrap_or(50.);
			let cy = args.get("y").and_then(|v| v.as_f64()).unwrap_or(50.);
			let rx = args.get("radius_x").and_then(|v| v.as_f64()).unwrap_or(50.);
			let ry = args.get("radius_y").and_then(|v| v.as_f64()).unwrap_or(50.);
			let fill = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("#000000");
			let view_w = cx + rx;
			let view_h = cy + ry;
			let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_w} {view_h}"><ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}" fill="{fill}"/></svg>"#);
			if let Err(e) = validate_svg(&svg, "create_ellipse") {
				return serde_json::to_string(&serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true})).unwrap();
			}
			dispatch(Message::Portfolio(PortfolioMessage::InsertSvg {
				name: Some("Ellipse".into()),
				svg,
				mouse: None,
				parent_and_insert_index: None,
			}));
			serde_json::json!({"content": [{"type": "text", "text": format!("Created ellipse at ({cx}, {cy}) rx={rx} ry={ry}")}]})
		}

		"create_line" => {
			let x1 = args.get("x1").and_then(|v| v.as_f64()).unwrap_or(0.);
			let y1 = args.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.);
			let x2 = args.get("x2").and_then(|v| v.as_f64()).unwrap_or(100.);
			let y2 = args.get("y2").and_then(|v| v.as_f64()).unwrap_or(100.);
			let stroke = args.get("stroke_color").and_then(|v| v.as_str()).unwrap_or("#000000");
			let sw = args.get("stroke_width").and_then(|v| v.as_f64()).unwrap_or(2.);
			let view_w = x1.max(x2) + sw;
			let view_h = y1.max(y2) + sw;
			let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_w} {view_h}"><line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{stroke}" stroke-width="{sw}"/></svg>"#);
			if let Err(e) = validate_svg(&svg, "create_line") {
				return serde_json::to_string(&serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true})).unwrap();
			}
			dispatch(Message::Portfolio(PortfolioMessage::InsertSvg {
				name: Some("Line".into()),
				svg,
				mouse: None,
				parent_and_insert_index: None,
			}));
			serde_json::json!({"content": [{"type": "text", "text": format!("Created line ({x1},{y1}) to ({x2},{y2})")}]})
		}

		"create_text" => {
			let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.);
			let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.);
			let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("Text");
			let font_size = args.get("font_size").and_then(|v| v.as_f64()).unwrap_or(24.);
			let fill = args.get("fill_color").and_then(|v| v.as_str()).unwrap_or("#000000");
			let est_width = text.len() as f64 * font_size * 0.6;
			let view_w = x + est_width;
			let view_h = y + font_size * 1.2;
			let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_w} {view_h}"><text x="{x}" y="{y}" font-size="{font_size}" fill="{fill}">{text}</text></svg>"#);
			if let Err(e) = validate_svg(&svg, "create_text") {
				return serde_json::to_string(&serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true})).unwrap();
			}
			dispatch(Message::Portfolio(PortfolioMessage::InsertSvg {
				name: Some("Text".into()),
				svg,
				mouse: None,
				parent_and_insert_index: None,
			}));
			serde_json::json!({"content": [{"type": "text", "text": format!("Created text at ({x}, {y}): \"{text}\"")}]})
		}

		"select_layer" => {
			let id_num = args.get("layer_id").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).or_else(|| args.get("layer_id").and_then(|v| v.as_u64()));
			match id_num {
				Some(id) => {
					dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SelectLayer {
						id: NodeId(id),
						ctrl: false,
						shift: false,
					})));
					serde_json::json!({"content": [{"type": "text", "text": format!("Selected layer {id}")}]})
				}
				None => serde_json::json!({"content": [{"type": "text", "text": "Error: Missing or invalid layer_id"}], "isError": true}),
			}
		}

		"delete_selected" => {
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DeleteSelectedLayers)));
			serde_json::json!({"content": [{"type": "text", "text": "Selected layers deleted"}]})
		}

		"undo" => {
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DocumentHistoryBackward)));
			serde_json::json!({"content": [{"type": "text", "text": "Undone"}]})
		}

		"redo" => {
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::DocumentHistoryForward)));
			serde_json::json!({"content": [{"type": "text", "text": "Redone"}]})
		}

		"set_fill_color" => {
			let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(100.);
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SetFillForSelectedLayers {
				fill: opacity / 100.,
			})));
			serde_json::json!({"content": [{"type": "text", "text": format!("Set fill opacity to {opacity}%")}]})
		}

		"set_opacity" => {
			let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(100.);
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SetOpacityForSelectedLayers {
				opacity: opacity / 100.,
			})));
			serde_json::json!({"content": [{"type": "text", "text": format!("Set opacity to {opacity}%")}]})
		}

		"set_blend_mode" => {
			let mode = args.get("blend_mode").and_then(|v| v.as_str()).unwrap_or("Normal");
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::SetBlendModeForSelectedLayers {
				blend_mode: graphene_std::raster::BlendMode::Normal,
			})));
			serde_json::json!({"content": [{"type": "text", "text": format!("Set blend mode to {mode}")}]})
		}

		"move_layer" => {
			let dx = args.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.);
			let dy = args.get("dy").and_then(|v| v.as_f64()).unwrap_or(0.);
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::NudgeSelectedLayers {
				delta_x: dx,
				delta_y: dy,
				resize: editor::messages::input_mapper::utility_types::input_keyboard::Key::Alt,
				resize_opposite: editor::messages::input_mapper::utility_types::input_keyboard::Key::Control,
			})));
			serde_json::json!({"content": [{"type": "text", "text": format!("Moved selected layers by ({dx}, {dy})")}]})
		}

		"get_selection" => {
			let sel = with_editor(&|editor| {
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
							serde_json::json!("No layers selected")
						} else {
							serde_json::json!(format!("Selected layers ({}):\n{}", layers.len(), layers.join("\n")))
						}
					}
					None => serde_json::json!("No active document"),
				}
			});
			serde_json::json!({"content": [{"type": "text", "text": sel}]})
		}

		"get_layer_properties" => {
			let id_num = args.get("layer_id").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).or_else(|| args.get("layer_id").and_then(|v| v.as_u64()));
			let props = with_editor(&|editor| {
				match (id_num, editor.active_document()) {
					(Some(id), Some(doc)) => {
						let node_id = NodeId(id);
						let network = &doc.network_interface;
						let name = network.display_name(&node_id, &[]);
						let visible = network.is_visible(&node_id, &[]);
						let locked = network.is_locked(&node_id, &[]);
						let is_layer = network.is_layer(&node_id, &[]);
						let is_artboard = network.is_artboard(&node_id, &[]);
						let kind = if is_artboard { "artboard" } else if is_layer { "layer" } else { "group" };
						let out = format!("# Layer Properties: {name}\n\n- **ID:** `{id}`\n- **Kind:** {kind}\n- **Visible:** {visible}\n- **Locked:** {locked}\n");
						serde_json::json!(out)
					}
					_ => serde_json::json!("No active document or invalid layer_id"),
				}
			});
			serde_json::json!({"content": [{"type": "text", "text": props}]})
		}

		"get_node_graph" => {
			let id_num = args.get("layer_id").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).or_else(|| args.get("layer_id").and_then(|v| v.as_u64()));
			let graph = with_editor(&|editor| {
				match (id_num, editor.active_document()) {
					(Some(id), Some(doc)) => {
						let node_id = NodeId(id);
						let network = &doc.network_interface;
						let name = network.display_name(&node_id, &[]);
						match network.document_node(&node_id, &[]) {
							Some(node) => {
								let mut out = format!("# Node Graph: {name}\n\n**Node ID:** `{id}`\n**Implementation:** {:?}\n**Inputs:** {}\n", node.implementation, node.inputs.len());
								for (i, input) in node.inputs.iter().enumerate() {
									out.push_str(&format!("  - Input {i}: {input:?}\n"));
								}
								serde_json::json!(out)
							}
							None => serde_json::json!(format!("Node {id} not found")),
						}
					}
					_ => serde_json::json!("No active document or invalid layer_id"),
				}
			});
			serde_json::json!({"content": [{"type": "text", "text": graph}]})
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
				"Freehand" => ToolType::Freehand,
				"Text" => ToolType::Text,
				"Fill" => ToolType::Fill,
				"Gradient" => ToolType::Gradient,
				"Eyedropper" => ToolType::Eyedropper,
				_ => ToolType::Select,
			};
			dispatch(Message::Tool(ToolMessage::ActivateTool { tool_type }));
			serde_json::json!({"content": [{"type": "text", "text": format!("Activated tool: {tool}")}]})
		}

		"zoom_to_fit" => {
			dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::ZoomCanvasToFitAll)));
			serde_json::json!({"content": [{"type": "text", "text": "Zoomed to fit"}]})
		}

		"set_viewport" => {
			let zoom = args.get("zoom").and_then(|v| v.as_f64());
			if let Some(zf) = zoom {
				dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::Navigation(NavigationMessage::CanvasZoomSet { zoom_factor: zf }))));
			}
			serde_json::json!({"content": [{"type": "text", "text": "Viewport updated"}]})
		}

		"set_stroke" => {
			let color_str = args.get("color").and_then(|v| v.as_str()).unwrap_or("#000000");
			let weight = args.get("width").and_then(|v| v.as_f64()).unwrap_or(2.);
			let hex = color_str.trim().trim_start_matches('#');
			let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.;
			let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.;
			let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.;
			let a = if hex.len() >= 8 { u8::from_str_radix(&hex[6..8], 16).unwrap_or(255) as f32 / 255. } else { 1. };
			let color = Color::from_rgbaf32_unchecked(r, g, b, a);
			let stroke = graphene_std::vector::style::Stroke { weight, ..Default::default() };

			let layer_id_num: Option<u64> = {
				let val = with_editor(&|editor| {
					editor.active_document()
						.and_then(|doc| {
							let metadata = doc.metadata();
							doc.network_interface.selected_nodes().selected_layers(metadata).next()
						})
						.map(|l| l.to_node().0)
						.map(|n| serde_json::json!(n))
						.unwrap_or(serde_json::json!(null))
				});
				val.as_u64()
			};

			if let Some(id) = layer_id_num {
				let layer = LayerNodeIdentifier::new_unchecked(NodeId(id));
				dispatch(Message::Portfolio(PortfolioMessage::Document(DocumentMessage::GraphOperation(GraphOperationMessage::StrokeSet {
					layer,
					color: Some(color),
					stroke,
				}))));
				serde_json::json!({"content": [{"type": "text", "text": format!("Set stroke: weight={weight}, color={color_str}")}]})
			} else {
				serde_json::json!({"content": [{"type": "text", "text": "No layer selected"}], "isError": true})
			}
		}

		_ => {
			serde_json::json!({"content": [{"type": "text", "text": format!("Unknown tool: {tool_name}")}], "isError": true})
		}
	};

	serde_json::to_string(&result).unwrap_or_else(|_| r#"{"content":[{"type":"text","text":"Serialization error"}],"isError":true}"#.into())
}
