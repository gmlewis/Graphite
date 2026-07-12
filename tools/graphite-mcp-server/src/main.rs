use clap::Parser;

#[derive(clap::Parser)]
#[clap(name = "graphite-mcp", version)]
struct Cli {
    /// Run in standalone mode (no editor, catalog-only tools)
    #[arg(long)]
    standalone: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    if cli.standalone {
        graphite_mcp_server::run_standalone().await
    } else {
        eprintln!("Error: --standalone flag required for standalone mode.");
        eprintln!("For headed mode, use the Graphite desktop app with --mcp flag.");
        std::process::exit(1);
    }
}
