mod editor_bridge;
mod mcp_protocol;
mod tools;

use mcp_protocol::{JsonRpcMessage, McpServer};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut writer = stdout;

    let mut server = McpServer::new();

    eprintln!("graphite-mcp-server started, waiting for JSON-RPC on stdin...");

    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            break;
        }

        let msg = match serde_json::from_str::<JsonRpcMessage>(buf.trim()) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Failed to parse JSON-RPC: {e}");
                continue;
            }
        };

        let response = server.handle(msg).await;
        if let Some(resp) = response {
            let mut resp_str = serde_json::to_string(&resp)?;
            resp_str.push('\n');
            writer.write_all(resp_str.as_bytes()).await?;
            writer.flush().await?;
        }
    }

    Ok(())
}
