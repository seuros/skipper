#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // stdout carries JSON-RPC; logs go to stderr, which MCP clients capture.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    skipper::mcp::run().await
}
