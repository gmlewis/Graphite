//! Lightweight MCP client proxy for Graphite.
//!
//! Connects to the Graphite desktop app's WebSocket server and bridges
//! MCP tool calls from stdin/stdout to the WebSocket.

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> Result<()> {
    let ws_url = std::env::var("GRAPHITE_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:8080".to_string());

    eprintln!("Connecting to Graphite at {ws_url}...");

    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    eprintln!("Connected. Forwarding MCP requests...");

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = io::stdout();

    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            break;
        }

        // Forward JSON-RPC to WebSocket
        if let Err(e) = write.send(Message::Text(buf.trim().to_string().into())).await {
            eprintln!("WebSocket write error: {e}");
            break;
        }

        // Read response from WebSocket
        match read.next().await {
            Some(Ok(Message::Text(text))) => {
                let mut resp_str = text.to_string();
                resp_str.push('\n');
                stdout.write_all(resp_str.as_bytes()).await?;
                stdout.flush().await?;
            }
            Some(Ok(Message::Close(_))) => {
                eprintln!("WebSocket closed by server");
                break;
            }
            Some(Err(e)) => {
                eprintln!("WebSocket read error: {e}");
                break;
            }
            None => {
                eprintln!("WebSocket stream ended");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
