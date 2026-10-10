use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, cli_error,
    mcp_tool,
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
    /// Watch every run of this commit instead (a sha, HEAD, a branch); returns
    /// once all have finished
    commit: Option<String>,
    /// Give up after this many seconds and return the current status
    /// (default: 60, clamped to 10..=300)
    wait_secs: Option<u64>,
    /// Seconds between polls (default: 10, clamped to 5..=60)
    poll_secs: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct BuildWatchResult {
    /// The watched run (run mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<BuildRun>,
    /// Commit mode: the commit's full sha and every run of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<Vec<BuildRun>>,
    /// Commit mode: failure | cancelled | pending | success | `no_runs`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<&'static str>,
    /// The status differs from when this call started.
    pub changed: bool,
    /// Finished (every run, in commit mode); calling again returns the same.
    pub terminal: bool,
    pub waited_secs: u64,
}

impl SkipperServer {
    #[mcp_tool(
        name = "build_watch",
        description = "Wait for a CI run's status to change, or with commit= for every run of a commit to finish, up to wait_secs. Returns the status either way; call again while terminal is false",
        task_support = "optional",
        output = "BuildWatchResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && (e.get_custom(\"forge:github\").is_some() || e.get_custom(\"forge:gitlab\").is_some())).unwrap_or(false)"
    )]
    async fn build_watch(&self, _ctx: Ctx<'_>, params: Parameters<BuildWatchParams>) -> ToolResult {
        let params = params.0;
        let provider = self.ci_provider(params.provider.as_deref())?;
        let wait = Duration::from_secs(params.wait_secs.unwrap_or(60).clamp(10, 300));
        let interval = Duration::from_secs(params.poll_secs.unwrap_or(10).clamp(5, 60));

        if let Some(rev) = params.commit {
            if params.run_id.is_some() {
                return Err(ToolError::InvalidArguments(
                    "pass run_id or commit, not both".to_string(),
                ));
            }
            return self.watch_commit(provider.as_ref(), &rev, wait, interval).await;
        }

        let initial =
            provider.ci_run(&self.env, params.run_id.as_deref()).await.map_err(cli_error)?;
        if initial.is_terminal() {
            return structured(BuildWatchResult::run(initial, false, 0));
        }

        let start = Instant::now();
        let deadline = start + wait;
        let initial_status = initial.status;
        let mut current = initial;
        while Instant::now() < deadline {
            let remaining = deadline - Instant::now();
            tokio::time::sleep(interval.min(remaining)).await;

            current = provider.ci_run(&self.env, Some(&current.id)).await.map_err(cli_error)?;
            if current.status != initial_status {
                break;
            }
        }

        let changed = current.status != initial_status;
        structured(BuildWatchResult::run(current, changed, start.elapsed().as_secs()))
    }

    /// Poll every run of `rev`'s commit until all have finished or `wait` ends.
    /// Runs registering late (right after a push) are picked up as they come.
    async fn watch_commit(
        &self,
        provider: &dyn crate::provider::Provider,
        rev: &str,
        wait: Duration,
        interval: Duration,
    ) -> ToolResult {
        use crate::environment::Environment as _;

        let sha = crate::git::commit_id(self.env.cwd(), rev)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let start = Instant::now();
        let deadline = start + wait;

        let initial = provider.ci_runs_for_commit(&self.env, &sha).await.map_err(cli_error)?;
        // Polled runs, kept apart from `initial` to tell whether they moved.
        let mut latest: Option<Vec<BuildRun>> = None;
        while !all_finished(latest.as_deref().unwrap_or(&initial)) && Instant::now() < deadline {
            let remaining = deadline - Instant::now();
            tokio::time::sleep(interval.min(remaining)).await;
            latest = Some(provider.ci_runs_for_commit(&self.env, &sha).await.map_err(cli_error)?);
        }
        let changed = latest.as_ref().is_some_and(|runs| *runs != initial);
        let runs = latest.unwrap_or(initial);

        structured(BuildWatchResult {
            run: None,
            changed,
            terminal: all_finished(&runs),
            conclusion: Some(crate::provider::conclusion_of(&runs)),
            commit: Some(sha),
            runs: Some(runs),
            waited_secs: start.elapsed().as_secs(),
        })
    }
}

impl BuildWatchResult {
    fn run(run: BuildRun, changed: bool, waited_secs: u64) -> Self {
        Self {
            terminal: run.is_terminal(),
            run: Some(run),
            commit: None,
            runs: None,
            conclusion: None,
            changed,
            waited_secs,
        }
    }
}

fn all_finished(runs: &[BuildRun]) -> bool {
    !runs.is_empty() && runs.iter().all(BuildRun::is_terminal)
}
