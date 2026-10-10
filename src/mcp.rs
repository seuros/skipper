pub mod git;
pub mod instructions;
pub mod resources;
pub mod tools;

pub use tools::SkipperServer;

use crate::config::{Config, WritesConfig};
use crate::environment::{Environment as _, SkipperEnvironment};
use crate::provider::Registry;
use crate::remote::ForgeHosts;
use crate::watcher::{GitStatusWatcher, RemoteWatcher, WatcherManager};
use mcp_host::prelude::*;
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct McpEnvironment {
    env: Arc<SkipperEnvironment>,
    registry: Arc<Registry>,
    writes: WritesConfig,
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

    /// Presence is the answer: every predicate asks `.is_some()`, and they
    /// run per tool on every listing. An empty `String` does not allocate.
    fn get_custom(&self, key: &str) -> Option<String> {
        let enabled = if key == "writes" {
            self.writes.enabled
        } else if let Some(name) = key.strip_prefix("forge:") {
            self.registry.is_enabled(name) && self.env.has_forge(name)
        } else {
            key.strip_prefix("provider:").is_some_and(|name| self.registry.is_enabled(name))
        };
        enabled.then(String::new)
    }
}

/// Re-reads the workspace's remotes before every `tools/list` and `tools/call`,
/// so forge tool visibility follows `git remote add` or a switched upstream at
/// once instead of on the next watcher tick. The watcher still notifies.
struct RemoteHydrator(Arc<SkipperEnvironment>);

impl CapabilityHydrator for RemoteHydrator {
    fn hydrate(&self, _ctx: CapabilityHydrationContext) -> CapabilityHydrationFuture<'_> {
        self.0.refresh();
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Forge tools fail fast while the forge is down: three upstream failures in a
/// minute open a tool's breaker for 30s. Caller errors (bad ref, no PR for the
/// branch) answer `InvalidArguments` and never count.
const fn breaker() -> ToolBreakerConfig {
    ToolBreakerConfig {
        failure_threshold: 3,
        failure_window_secs: 60.0,
        half_open_timeout_secs: 30.0,
        success_threshold: 1,
        call_timeout_secs: 120.0,
    }
}

/// Requests per second, and burst, across the server: far above an agent's
/// pace, low enough to stop a runaway loop before it spends the forge's API
/// rate limit.
const RATE_LIMIT: (f64, usize) = (20.0, 40);

pub async fn build_server() -> std::io::Result<(Server, Arc<WatcherManager>)> {
    let config = Config::load();
    let registry = Arc::new(Registry::with_defaults(|forge| config.is_disabled(forge)));
    registry.detect_all().await;

    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&config.hosts);

    let cwd = std::env::current_dir()?;
    let env = Arc::new(SkipperEnvironment::with_hosts(&cwd, hosts));
    let has_repo = env.has_git_repo();

    tracing::info!(
        forges = ?env.forges(),
        unmapped_hosts = ?env.unknown_hosts(),
        "resolved forges from git remotes"
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let manager = Arc::new(WatcherManager::new(tx));

    let server = Server::builder("skipper", env!("CARGO_PKG_VERSION"))
        .with_instructions(instructions::BASE)
        .with_instructions_provider(instructions::for_client)
        .with_resource_errors_as_content(instructions::HIDES_RESOURCE_ERRORS)
        .map_err(std::io::Error::other)?
        .with_tools(true)
        .with_resources(false, false)
        .with_resource_templates()
        .with_tasks(true, true)
        .with_circuit_breaker(breaker())
        .with_rate_limit(RATE_LIMIT.0, RATE_LIMIT.1)
        .with_capability_hydrator(RemoteHydrator(env.clone()))
        .with_logging()
        .with_environment(McpEnvironment {
            env: env.clone(),
            registry: registry.clone(),
            writes: config.writes,
        })
        .build();

    server.register_router(
        tools::router(),
        Arc::new(SkipperServer {
            #[cfg(any(feature = "github", feature = "gitlab"))]
            registry: registry.clone(),
            #[cfg(feature = "tea")]
            cwd: cwd.clone(),
            env: env.clone(),
            writes: config.writes,
            #[cfg(feature = "github")]
            pr_watcher: crate::pr_watch::PrWatcher::new(env.clone()),
        }),
    );

    server.register_router(git::router(), Arc::new(crate::git::GitServer));

    let sender = server.notification_sender();

    // A probe that timed out at startup (slow `gh auth status`, flaky network)
    // hides that forge's tools; retry and surface them once it answers.
    if !registry.unreachable().is_empty() {
        let sender = sender.clone();
        tokio::spawn(async move {
            registry
                .retry_unreachable(|online| {
                    tracing::info!(providers = ?online, "providers reachable; refreshing tools");
                    if let Err(e) = sender
                        .send(JsonRpcNotification::new("notifications/tools/list_changed", None))
                    {
                        tracing::warn!(error = %e, "dropped tools/list_changed notification");
                    }
                })
                .await;
        });
    }

    if has_repo {
        manager.add(Arc::new(GitStatusWatcher::new(&cwd)));
    }

    manager.add(Arc::new(RemoteWatcher::new(env)));

    tokio::spawn(async move {
        while let Some(n) = rx.recv().await {
            if n.watcher == RemoteWatcher::NAME
                && let Err(e) =
                    sender.send(JsonRpcNotification::new("notifications/tools/list_changed", None))
            {
                tracing::warn!(error = %e, "dropped tools/list_changed notification");
            }

            // Moved, not `json!`: that macro serializes through a reference
            // and would deep-copy `data`.
            let params = Map::from_iter([
                ("level".to_owned(), Value::from("info")),
                ("logger".to_owned(), Value::from(n.watcher)),
                ("data".to_owned(), n.data),
                ("message".to_owned(), Value::String(n.message)),
            ]);
            if let Err(e) = sender.send(JsonRpcNotification::new(
                "notifications/message",
                Some(Value::Object(params)),
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
    manager.stop();
    result
}
