use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, cli_error,
    mcp_tool,
};
use crate::error::CliError;
use crate::provider::github::{CheckCounts, GitHubProvider, PrCheck};
use mcp_host::prelude::structured;
use serde::Serialize;
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
pub struct PrBuildWaitParams {
    /// PR number (default: the PR belonging to the current branch)
    pr: Option<u64>,
    /// Stop watching at the first failing check (default: true)
    fail_fast: Option<bool>,
    /// Give up after this many seconds and report the pending snapshot
    /// (default: 1800, clamped to 30..=3600)
    timeout_secs: Option<u64>,
    /// Seconds between check polls (default: 10, clamped to 5..=60)
    poll_secs: Option<u64>,
}

/// One failing check, surfaced first so the model can act on it directly.
#[derive(Serialize, JsonSchema)]
pub struct FailedCheck {
    pub name: String,
    /// Owning workflow; empty for commit statuses.
    pub workflow: String,
    pub link: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct PrBuildResult {
    /// success | failure | cancelled | pending (pending = timed out waiting)
    pub conclusion: String,
    /// True when timeout_secs elapsed before the checks finished.
    pub timed_out: bool,
    pub counts: CheckCounts,
    pub failed: Vec<FailedCheck>,
    pub checks: Vec<PrCheck>,
}

impl PrBuildResult {
    fn from_checks(checks: Vec<PrCheck>, timed_out: bool) -> Self {
        let counts = CheckCounts::tally(&checks);
        let failed = checks
            .iter()
            .filter(|c| c.bucket == "fail")
            .map(|c| FailedCheck {
                name: c.name.clone(),
                workflow: c.workflow.clone(),
                link: c.link.clone(),
                description: c.description.clone(),
            })
            .collect();

        Self { conclusion: counts.conclusion().to_string(), timed_out, counts, failed, checks }
    }
}

impl SkipperServer {
    #[mcp_tool(
        name = "pr_build_wait",
        description = "Block until a PR's checks conclude. Long-running; prefer task execution. Unregistered checks count as pending; times out to a pending snapshot",
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
                "GitHub CLI is not available (missing or unauthenticated)".to_string(),
            ));
        }

        let pr = params.0.pr;
        let fail_fast = params.0.fail_fast.unwrap_or(true);
        let timeout = Duration::from_secs(params.0.timeout_secs.unwrap_or(1800).clamp(30, 3600));
        let interval = Duration::from_secs(params.0.poll_secs.unwrap_or(10).clamp(5, 60));

        let gh = GitHubProvider::new();
        match gh.pr_checks_watch(pr, fail_fast, timeout, interval).await {
            Ok(checks) => structured(PrBuildResult::from_checks(checks, false)),
            Err(CliError::Timeout { .. }) => {
                let checks = gh.pr_checks(pr).await.map_err(cli_error)?;
                structured(PrBuildResult::from_checks(checks, true))
            }
            Err(e) => Err(cli_error(e)),
        }
    }
}
