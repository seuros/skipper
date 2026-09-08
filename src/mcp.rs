pub mod git;
pub mod resources;
pub mod tools;

pub use tools::SkipperServer;

use crate::config::Config;
use crate::environment::{Environment as _, SkipperEnvironment};
use crate::provider::Registry;
use crate::remote::ForgeHosts;
use crate::watcher::{GitStatusWatcher, RemoteWatcher, WatcherManager};

const REMOTE_WATCHER: &str = "remotes";
use mcp_host::prelude::*;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub const SERVER_INSTRUCTIONS: &str = "\
Use skipper for git and forge work in this workspace: local git, repository
metadata, CI runs, and pull request checks. Prefer it over running git, gh,
glab, or tea yourself.

Resources, read by URI:

  skipper://repo                 the repository this workspace points at
  skipper://pr/{number}/checks   a PR's checks grouped by workflow;
                                 use `current` for this branch's PR";

pub struct McpEnvironment {
    env: Arc<SkipperEnvironment>,
    providers: Vec<&'static str>,
}

impl Environment for McpEnvironment {
    fn has_git_repo(&self) -> bool {
        self.env.has_git_repo()
    }

    fn git_is_clean(&self) -> bool {
        self.env.git_is_clean()
    }

    fn git_has_staged(&self) -> bool {
        self.env.git_has_staged()
    }

    fn cwd(&self) -> &Path {
        self.env.cwd()
    }

    fn get_custom(&self, key: &str) -> Option<String> {
        if let Some(name) = key.strip_prefix("forge:") {
            return (self.providers.contains(&name) && self.env.forges().contains(name))
                .then(|| "enabled".to_string());
        }

        key.strip_prefix("provider:")
            .filter(|name| self.providers.contains(name))
            .map(|_| "enabled".to_string())
    }
}

pub async fn build_server() -> std::io::Result<(Server, Arc<WatcherManager>)> {
    let mut registry = Registry::with_defaults();
    registry.detect_all().await;
    let providers = registry.enabled_names();

    let config = Config::load();
    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&config.hosts);

    let cwd = std::env::current_dir()?;
    let env = Arc::new(SkipperEnvironment::with_hosts(&cwd, hosts).await);
    let has_repo = env.has_git_repo();

    tracing::info!(
        forges = ?env.forges(),
        unmapped_hosts = ?env.unknown_hosts(),
        "resolved forges from git remotes"
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let manager = Arc::new(WatcherManager::new(tx));

    let server = Server::builder("skipper", env!("CARGO_PKG_VERSION"))
        .with_instructions(SERVER_INSTRUCTIONS)
        .with_tools(true)
        .with_resources(false, false)
        .with_resource_templates()
        .with_tasks(true, true)
        .with_logging()
        .with_environment(McpEnvironment { env: env.clone(), providers })
        .build();

    server.register_router(
        tools::router(),
        Arc::new(SkipperServer {
            #[cfg(any(feature = "github", feature = "gitlab"))]
            registry,
            #[cfg(feature = "tea")]
            cwd: cwd.clone(),
            #[cfg(any(feature = "github", feature = "gitlab"))]
            env: env.clone(),
        }),
    );

    server.register_router(git::router(), Arc::new(crate::git::GitServer));

    let sender = server.notification_sender();

    if has_repo {
        manager.add(Arc::new(GitStatusWatcher::new(&cwd))).await;
    }

    manager.add(Arc::new(RemoteWatcher::new(env))).await;

    tokio::spawn(async move {
        while let Some(n) = rx.recv().await {
            if n.watcher == REMOTE_WATCHER
                && let Err(e) =
                    sender.send(JsonRpcNotification::new("notifications/tools/list_changed", None))
            {
                tracing::warn!(error = %e, "dropped tools/list_changed notification");
            }

            if let Err(e) = sender.send(JsonRpcNotification::new(
                "notifications/message",
                Some(json!({
                    "level": "info",
                    "logger": n.watcher,
                    "data": n.data,
                    "message": n.message,
                })),
            )) {
                tracing::warn!(watcher = n.watcher, error = %e, "dropped watcher notification");
            }
        }
    });

    Ok((server, manager))
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (server, manager) = build_server().await?;
    let result = server.run(StdioTransport::new()).await;
    manager.stop().await;
    result
}
