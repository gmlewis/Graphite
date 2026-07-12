#[derive(clap::Parser)]
#[clap(name = "graphite", version)]
pub struct Cli {
	#[arg(help = "Files to open on startup")]
	pub files: Vec<std::path::PathBuf>,

	#[arg(long, action = clap::ArgAction::SetTrue, help = "Disable hardware accelerated UI rendering")]
	pub disable_ui_acceleration: bool,

	#[arg(long, action = clap::ArgAction::SetTrue, help = "Start MCP WebSocket server for AI agent control")]
	pub mcp_server: bool,

	#[arg(long, default_value = "8080", help = "Port for MCP WebSocket server (used with --mcp-server)")]
	pub mcp_port: u16,

	#[arg(long, action = clap::ArgAction::SetTrue, help = "Start headless MCP server on stdin/stdout (no window)")]
	pub mcp: bool,
}
