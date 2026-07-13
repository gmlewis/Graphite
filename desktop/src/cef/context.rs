#[cfg(not(target_os = "macos"))]
mod multithreaded;
mod singlethreaded;

mod builder;
pub(crate) use builder::{CefContextBuilder, InitError};

pub(crate) trait CefContext {
	fn work(&mut self);

	fn handle_window_event(&mut self, event: &winit::event::WindowEvent);

	fn notify_view_info_changed(&self);

	fn send_web_message(&self, message: Vec<u8>);
}

/// Null CEF context for MCP mode — all methods are no-ops.
pub(crate) struct NullCefContext;

impl CefContext for NullCefContext {
	fn work(&mut self) {}
	fn handle_window_event(&mut self, _event: &winit::event::WindowEvent) {}
	fn notify_view_info_changed(&self) {}
	fn send_web_message(&self, _message: Vec<u8>) {}
}
