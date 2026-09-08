use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolResult, cli_error, mcp_tool,
};
use crate::provider::BuildRun;
use mcp_host::prelude::structured;
use serde::Serialize;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Deserialize, JsonSchema)]
pub struct BuildWatchParams {
    /// Forge to query: "github" or "gitlab" (auto-detected when only one is enabled)
    provider: Option<String>,
    /// Run/pipeline id to watch (default: the latest run for the current branch)
    run_id: Option<String>,
    /// Give up after this many seconds and return the current status
    /// (default: 60, clamped to 10..=300)
    wait_secs: Option<u64>,
    /// Seconds between polls (default: 10, clamped to 5..=60)
    poll_secs: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct BuildWatchResult {
    pub run: BuildRun,
    /// The status differs from when this call started.
    pub changed: bool,
    /// The run has finished; calling again would return the same thing.
    pub terminal: bool,
    pub waited_secs: u64,
}

impl SkipperServer {
    #[mcp_tool(
        name = "build_watch",
        description = "Wait for a CI run's status to change, up to wait_secs. Returns the status either way; call again while terminal is false",
        output = "BuildWatchResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && (e.get_custom(\"forge:github\").is_some() || e.get_custom(\"forge:gitlab\").is_some())).unwrap_or(false)"
    )]
    async fn build_watch(&self, _ctx: Ctx<'_>, params: Parameters<BuildWatchParams>) -> ToolResult {
        let provider = self.ci_provider(params.0.provider.as_deref())?;
        let initial = provider.ci_run(params.0.run_id.as_deref()).await.map_err(cli_error)?;

        if initial.is_terminal() {
            return structured(BuildWatchResult {
                terminal: true,
                changed: false,
                waited_secs: 0,
                run: initial,
            });
        }

        let wait = Duration::from_secs(params.0.wait_secs.unwrap_or(60).clamp(10, 300));
        let interval = Duration::from_secs(params.0.poll_secs.unwrap_or(10).clamp(5, 60));
        let start = Instant::now();
        let deadline = start + wait;

        let mut current = initial.clone();
        while Instant::now() < deadline {
            let remaining = deadline - Instant::now();
            tokio::time::sleep(interval.min(remaining)).await;

            current = provider.ci_run(Some(&initial.id)).await.map_err(cli_error)?;
            if current.status != initial.status {
                break;
            }
        }

        structured(BuildWatchResult {
            changed: current.status != initial.status,
            terminal: current.is_terminal(),
            waited_secs: start.elapsed().as_secs(),
            run: current,
        })
    }
}
