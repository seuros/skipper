use crate::environment::{Environment as _, SkipperEnvironment};
use crate::error::{CliError, Result};
use crate::executor::{self};
use crate::provider::{BoxFuture, BuildRun, Provider, retryable, retrying};
use crate::version::minimum;
use crate::workspace::ForgeRepo;
use schemars::JsonSchema;
use semver::Version;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

mod checks;
mod client;
mod discussion;
mod issues;
mod pulls;
mod runs;

pub use client::ApiResponse;
pub use discussion::{PrDiscussion, PrNote};
pub(crate) use discussion::{User, login};
pub use pulls::{PrFile, PrOverview};
pub use runs::OwnedRepo;

#[derive(Deserialize)]
struct AuthReport {
    hosts: std::collections::HashMap<String, Vec<AuthEntry>>,
}

#[derive(Deserialize)]
struct AuthEntry {
    state: String,
    error: Option<String>,
}

impl AuthReport {
    /// `Err` when nothing succeeded but a check failed on the network (gh
    /// reports resets as `error`, not `timeout`): login unknown.
    fn logged_in(self) -> Result<bool> {
        let entries: Vec<AuthEntry> = self.hosts.into_values().flatten().collect();
        if entries.iter().any(|e| e.state == "success") {
            return Ok(true);
        }
        let unreachable = |e: &AuthEntry| {
            e.state == "timeout" || e.error.as_deref().is_some_and(super::network_failure)
        };
        match entries.into_iter().find(unreachable) {
            Some(e) => Err(CliError::execution_failed(
                "gh",
                1,
                e.error.unwrap_or_else(|| "auth check timed out".to_string()),
            )),
            None => Ok(false),
        }
    }
}

/// GitHub over its REST and GraphQL APIs. `gh` only hands out the token.
pub struct GitHubProvider {
    min_version: Version,
}

impl GitHubProvider {
    pub fn new() -> Self {
        Self { min_version: minimum::github() }
    }
}

impl Default for GitHubProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for GitHubProvider {
    fn name(&self) -> &'static str {
        "github"
    }

    fn cli(&self) -> &'static str {
        "gh"
    }

    fn min_version(&self) -> Version {
        self.min_version.clone()
    }

    fn check_auth(&self) -> BoxFuture<'_, crate::error::Result<bool>> {
        Box::pin(async move {
            match client::get("github.com", "user", None).await {
                Ok(response) if response.status == 200 => Ok(true),
                Ok(response) if matches!(response.status, 401 | 403) => Ok(false),
                Ok(response) => Err(response.error("auth check")),
                // No github.com login; gh may still hold an Enterprise one.
                Err(CliError::AuthRequired { .. }) => self.any_login().await,
                Err(e) => Err(e),
            }
        })
    }

    fn ci_runs<'a>(
        &'a self,
        env: &'a SkipperEnvironment,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move {
            let repo = crate::workspace::forge_repo_on(env, "github")?;
            self.runs(&repo, None, limit).await
        })
    }

    fn ci_runs_for_commit<'a>(
        &'a self,
        env: &'a SkipperEnvironment,
        sha: &'a str,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move {
            let repo = crate::workspace::forge_repo_on(env, "github")?;
            self.commit_runs(&repo, sha).await
        })
    }

    fn ci_run<'a>(
        &'a self,
        env: &'a SkipperEnvironment,
        id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<BuildRun>> {
        Box::pin(async move {
            let repo = crate::workspace::forge_repo_on(env, "github")?;
            if let Some(id) = id {
                return self.run(&repo, id).await;
            }
            let branch = crate::git::repo_info(env.cwd()).ok().and_then(|info| info.branch);
            self.runs(&repo, branch.as_deref(), 1).await?.pop().ok_or_else(|| {
                CliError::no_target(match &branch {
                    Some(branch) => format!("no CI runs for {branch} in {}", repo.full_name()),
                    None => format!("no CI runs in {}", repo.full_name()),
                })
            })
        })
    }
}

impl GitHubProvider {
    /// GET a REST `path` on `host`, conditional on `etag` when given. Any HTTP
    /// status comes back as `Ok`; only a failure to get a response is an `Err`.
    pub async fn api_get(&self, host: &str, path: &str, etag: Option<&str>) -> Result<ApiResponse> {
        client::get(host, path, etag).await
    }

    /// `pr`, or the PR of the checked-out branch.
    pub async fn resolve_pr(&self, repo: &ForgeRepo, pr: Option<u64>, cwd: &Path) -> Result<u64> {
        if let Some(number) = pr {
            return Ok(number);
        }
        let branch = crate::git::repo_info(cwd)
            .ok()
            .and_then(|info| info.branch)
            .ok_or_else(|| CliError::no_target("detached HEAD; pass the PR number"))?;
        self.pr_for_branch(repo, &branch).await
    }

    pub async fn pr_checks_watch(
        &self,
        repo: &ForgeRepo,
        pr: u64,
        fail_fast: bool,
        timeout: Duration,
        interval: Duration,
    ) -> Result<ChecksWatch> {
        watch_checks(|| self.pr_checks(repo, pr), fail_fast, timeout, interval).await
    }

    /// `gh auth status` for logins on any host, Enterprise included.
    async fn any_login(&self) -> Result<bool> {
        let args = ["auth", "status", "--active", "--json", "hosts"];
        match executor::execute_success(self.cli(), &args, Duration::from_secs(10)).await {
            Ok(output) => output.json::<AuthReport>(self.cli())?.logged_in(),
            Err(CliError::ExecutionFailed { .. }) => super::cli_authenticated(self.cli()).await,
            Err(e) => Err(e),
        }
    }

    async fn api_json_at<T: DeserializeOwned>(&self, host: &str, path: &str) -> Result<T> {
        retrying(|| client::get(host, path, None)).await?.ok_json(path)
    }

    async fn graphql<T: DeserializeOwned>(
        &self,
        host: &str,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T> {
        retrying(|| client::graphql(host, query, variables.clone())).await
    }
}

/// REST page size: GitHub's maximum `per_page`.
pub(crate) const PAGE: usize = 100;

/// gh's bucket for a check run: pass | fail | pending | skipping | cancel.
/// Takes REST (lowercase) and GraphQL (uppercase) values alike.
pub(crate) fn run_bucket(status: &str, conclusion: Option<&str>) -> &'static str {
    if !status.eq_ignore_ascii_case("completed") {
        return "pending";
    }
    match conclusion.map(str::to_ascii_lowercase).as_deref() {
        Some("success") => "pass",
        Some("skipped" | "neutral") => "skipping",
        Some("cancelled") => "cancel",
        Some("stale") => "pending",
        _ => "fail",
    }
}

/// gh's bucket for a commit status.
pub(crate) fn status_bucket(state: &str) -> &'static str {
    match state.to_ascii_lowercase().as_str() {
        "success" => "pass",
        "pending" | "expected" => "pending",
        _ => "fail",
    }
}

/// How a check watch ended: the last snapshot it got, and whether the
/// deadline passed before the checks settled.
#[derive(Debug)]
pub struct ChecksWatch {
    pub checks: Vec<PrCheck>,
    pub timed_out: bool,
}

/// Poll until the checks settle, the first one fails (with `fail_fast`), or
/// `timeout` passes. A poll lost to the network or to its own timeout says
/// nothing about the checks: the watch keeps its last snapshot and polls
/// again, failing only when no poll got through before the deadline.
pub(crate) async fn watch_checks<F, Fut>(
    mut poll: F,
    fail_fast: bool,
    timeout: Duration,
    interval: Duration,
) -> Result<ChecksWatch>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<PrCheck>>>,
{
    let start = tokio::time::Instant::now();
    let deadline = start + timeout;
    let registration_grace = Duration::from_secs(120).min(timeout);
    let mut last = None;

    loop {
        let error = match poll().await {
            Ok(checks) => {
                let counts = CheckCounts::tally(&checks);
                let settled = if checks.is_empty() {
                    start.elapsed() >= registration_grace
                } else {
                    counts.pending == 0 || (fail_fast && counts.fail > 0)
                };
                if settled {
                    return Ok(ChecksWatch { checks, timed_out: false });
                }
                last = Some(checks);
                None
            }
            Err(e) if transient(&e) => {
                tracing::debug!(error = %e, "check poll failed transiently; polling again");
                Some(e)
            }
            Err(e) => return Err(e),
        };

        let now = tokio::time::Instant::now();
        if now >= deadline {
            return match last {
                Some(checks) => Ok(ChecksWatch { checks, timed_out: true }),
                None => Err(error.expect("a watch with no snapshot ended on a failed poll")),
            };
        }
        tokio::time::sleep(interval.min(deadline - now)).await;
    }
}

/// A failure that says nothing about what was asked: the network dropped,
/// GitHub stumbled, or the call itself ran out of time.
fn transient(e: &CliError) -> bool {
    matches!(e, CliError::Timeout { .. }) || retryable(e)
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrState {
    pub pr: u64,
    pub title: String,
    /// Head branch.
    pub head: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
}

/// One check on a PR's head commit.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrCheck {
    pub name: String,
    /// gh's classification: pass | fail | pending | skipping | cancel
    pub bucket: String,
    /// Owning workflow; absent for commit statuses.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow: String,
    /// The check's page, where its logs are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Checks tallied by bucket.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct CheckCounts {
    pub pass: u32,
    pub fail: u32,
    pub pending: u32,
    pub skipped: u32,
    pub cancelled: u32,
}

impl CheckCounts {
    pub fn tally(checks: &[PrCheck]) -> Self {
        let mut counts = Self::default();
        for check in checks {
            counts.add(&check.bucket);
        }
        counts
    }

    pub fn add(&mut self, bucket: &str) {
        match bucket {
            "pass" => self.pass += 1,
            "fail" => self.fail += 1,
            "skipping" => self.skipped += 1,
            "cancel" => self.cancelled += 1,
            _ => self.pending += 1,
        }
    }

    pub fn total(&self) -> u32 {
        self.pass + self.fail + self.pending + self.skipped + self.cancelled
    }

    pub fn conclusion(&self) -> &'static str {
        if self.total() == 0 {
            "no_checks"
        } else if self.fail > 0 {
            "failure"
        } else if self.cancelled > 0 {
            "cancelled"
        } else if self.pending > 0 {
            "pending"
        } else {
            "success"
        }
    }
}

#[cfg(test)]
mod tests;
