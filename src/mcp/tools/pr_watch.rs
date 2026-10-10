use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolResult, cli_error, mcp_tool,
};
use crate::pr_watch::{EventKind, EventKinds, PrEvent, PrSnapshot};
use mcp_host::prelude::structured;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
pub struct PrWatchParams {
    /// PR number (default: this branch's PR)
    pr: Option<u64>,
    /// Repo to read: a remote name or owner/name (default: the current remote)
    repo: Option<String>,
    /// Wake on these besides merged/closed (default: all). [] = merge/close only
    until: Option<Vec<EventKind>>,
    /// Max seconds to block (default: 600, clamped to 10..=3600)
    wait_secs: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct PrWatchResult {
    /// PR this call added or updated.
    pub pr: u64,
    /// false: another `pr_watch` call is blocking and will report this PR too.
    pub blocked: bool,
    /// Events on any watched PR, oldest first. Merged/closed/error end that PR's watch.
    pub events: Vec<PrEvent>,
    /// `wait_secs` ran out; the watch continues. Call again to keep waiting.
    pub timed_out: bool,
    /// PRs still watched.
    pub watching: Vec<Arc<PrSnapshot>>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "pr_watch",
        description = "Watch GitHub PRs: merge, close, comments, reviews, checks, pushes. One call blocks until an event on any watched PR; calling while one blocks just adds the PR. Non-blocking state: skipper://watch. Polls with ETags and backoff. Long-running; prefer task execution",
        task_support = "optional",
        output = "PrWatchResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.has_custom(\"forge:github\")).unwrap_or(false)"
    )]
    async fn pr_watch(&self, _ctx: Ctx<'_>, params: Parameters<PrWatchParams>) -> ToolResult {
        self.require_github()?;

        let until = params.0.until.map_or(EventKinds::ALL, EventKinds::from_iter);
        let (pr, _) = self
            .pr_watcher
            .add(params.0.pr, params.0.repo.as_deref(), until)
            .await
            .map_err(cli_error)?;

        let Some(blocker) = self.pr_watcher.try_block() else {
            return structured(PrWatchResult {
                pr,
                blocked: false,
                events: Vec::new(),
                timed_out: false,
                watching: self.pr_watcher.statuses(),
            });
        };

        let wait = Duration::from_secs(params.0.wait_secs.unwrap_or(600).clamp(10, 3600));
        let (events, timed_out) = blocker.wait(wait).await;
        drop(blocker);

        structured(PrWatchResult {
            pr,
            blocked: true,
            events,
            timed_out,
            watching: self.pr_watcher.statuses(),
        })
    }
}
