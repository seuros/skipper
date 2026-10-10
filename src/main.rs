//! skipper-mcp: git and forge operations for an MCP client, over stdio.

// Dead code is checked on the full build, the one that ships. A build with
// some forges left out also leaves out the callers of helpers they share.
#![cfg_attr(
    not(all(feature = "github", feature = "gitlab", feature = "tea")),
    allow(dead_code, dead_code_pub_in_binary, unused_imports)
)]

mod ascii;
mod config;
mod environment;
mod error;
mod executor;
mod git;
mod mcp;
#[cfg(feature = "github")]
mod pr_watch;
mod provider;
mod remote;
mod version;
mod watcher;
mod workspace;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // stdout carries JSON-RPC; logs go to stderr, which MCP clients capture.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    mcp::run().await
}
