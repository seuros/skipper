use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, cli_error,
    mcp_tool,
};
use crate::provider::github::{CheckCounts, GitHubProvider, PrCheck};
use mcp_host::prelude::structured;
use serde::Serialize;
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
pub struct PrBuildWaitParams {
    /// PR number (default: the PR belonging to the current branch)
    pr: Option<u64>,
    /// Repo to read: a remote name or owner/name (default: the current remote)
    repo: Option<String>,
    /// Stop watching at the first failing check (default: true)
    fail_fast: Option<bool>,
    /// Give up after this many seconds and report the pending snapshot
    /// (default: 1800, clamped to 30..=3600)
    timeout_secs: Option<u64>,
    /// Seconds between check polls (default: 10, clamped to 5..=60)
    poll_secs: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct PrBuildResult {
    /// success | failure | cancelled | pending (pending = timed out waiting)
    pub conclusion: &'static str,
    /// True when `timeout_secs` elapsed before the checks finished.
    pub timed_out: bool,
    pub counts: CheckCounts,
    /// Failed and cancelled checks, with links to their logs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<PrCheck>,
    /// Names of checks still queued or running.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
}

impl PrBuildResult {
    /// Passed and skipped checks only add to `counts`.
    pub(crate) fn from_checks(checks: Vec<PrCheck>, timed_out: bool) -> Self {
        let counts = CheckCounts::tally(&checks);
        let mut failed = Vec::new();
        let mut pending = Vec::new();
        for check in checks {
            match check.bucket {
                "pass" | "skipping" => {}
                "fail" | "cancel" => failed.push(check),
                _ => pending.push(check.name),
            }
        }

        Self { conclusion: counts.conclusion(), timed_out, counts, failed, pending }
    }
}

impl SkipperServer {
    #[mcp_tool(
        name = "pr_build_wait",
        description = "Block until a PR's checks conclude. Long-running; prefer task execution. Unregistered checks count as pending; network blips are retried; times out to a pending snapshot",
        task_support = "optional",
        output = "PrBuildResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:github\").is_some()).unwrap_or(false)"
    )]
    async fn pr_build_wait(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<PrBuildWaitParams>,
    ) -> ToolResult {
        if !self.registry.is_enabled("github") {
            return Err(ToolError::Execution(
                "GitHub is not available (gh missing or not logged in)".to_string(),
            ));
        }

        let gh = GitHubProvider::new();
        let (repo, pr) = gh
            .locate_pr(&self.env, params.0.repo.as_deref(), params.0.pr)
            .await
            .map_err(cli_error)?;
        let fail_fast = params.0.fail_fast.unwrap_or(true);
        let timeout = Duration::from_secs(params.0.timeout_secs.unwrap_or(1800).clamp(30, 3600));
        let interval = Duration::from_secs(params.0.poll_secs.unwrap_or(10).clamp(5, 60));

        let watch =
            gh.pr_checks_watch(&repo, pr, fail_fast, timeout, interval).await.map_err(cli_error)?;
        structured(PrBuildResult::from_checks(watch.checks, watch.timed_out))
    }
}
