use crate::environment::{Environment as _, SkipperEnvironment};
use crate::error::{CliError, Result};
use crate::executor::{self};
use crate::provider::{BoxFuture, BuildRun, CiTarget, Provider, is_zero, retryable, retrying};
use crate::version::minimum;
use crate::workspace::ForgeRepo;
use schemars::JsonSchema;
use semver::Version;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::Duration;

mod checks;
mod client;
mod discussion;
mod forks;
mod issues;
pub(crate) mod pulls;
mod runs;

pub use client::ApiResponse;
pub(crate) use client::rest_url;
pub use discussion::PrDiscussion;
pub(crate) use discussion::{User, login};
pub use pulls::{PrFile, PrOverview};

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
        let mut unreachable = None;
        for entry in self.hosts.into_values().flatten() {
            if entry.state == "success" {
                return Ok(true);
            }
            if unreachable.is_none()
                && (entry.state == "timeout"
                    || entry.error.as_deref().is_some_and(super::network_failure))
            {
                unreachable = Some(entry);
            }
        }
        match unreachable {
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
    pub const fn new() -> Self {
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
            match client::get("github.com", "https://api.github.com/user", None).await {
                Ok(response) if response.status == 200 => Ok(true),
                Ok(response) if matches!(response.status, 401 | 403) => Ok(false),
                Ok(response) => Err(response.error("auth check")),
                // No github.com login; gh may still hold an Enterprise one.
                Err(CliError::AuthRequired { .. }) => self.any_login().await,
                Err(e) => Err(e),
            }
        })
    }

    fn ci_target(&self, env: &SkipperEnvironment) -> Result<CiTarget> {
        Ok(CiTarget::Repo(crate::workspace::forge_repo_on(env, "github")?))
    }

    fn ci_runs<'a>(
        &'a self,
        target: &'a CiTarget,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move { self.runs(repo_of(target)?, None, limit).await })
    }

    fn ci_runs_for_commit<'a>(
        &'a self,
        target: &'a CiTarget,
        sha: &'a str,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move { self.commit_runs(repo_of(target)?, sha).await })
    }

    fn ci_run<'a>(
        &'a self,
        env: &'a SkipperEnvironment,
        target: &'a CiTarget,
        id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<BuildRun>> {
        Box::pin(async move {
            let repo = repo_of(target)?;
            if let Some(id) = id {
                return self.run(repo, id).await;
            }
            let branch = crate::git::current_branch(env.cwd()).ok().flatten();
            self.runs(repo, branch.as_deref(), 1).await?.pop().ok_or_else(|| {
                CliError::no_target(match &branch {
                    Some(branch) => format!("no CI runs for {branch} in {}", repo.full_name()),
                    None => format!("no CI runs in {}", repo.full_name()),
                })
            })
        })
    }
}

/// The repo [`GitHubProvider::ci_target`] resolved; a target from another
/// forge is a caller's mistake.
fn repo_of(target: &CiTarget) -> Result<&ForgeRepo> {
    match target {
        CiTarget::Repo(repo) => Ok(repo),
        CiTarget::Workspace => Err(CliError::no_target("no GitHub repository resolved for CI")),
    }
}

impl GitHubProvider {
    /// GET a REST `url` ([`rest_url`]) on `host`, conditional on `etag` when
    /// given. Any HTTP status comes back as `Ok`; only a failure to get a
    /// response is an `Err`.
    pub async fn api_get(&self, host: &str, url: &str, etag: Option<&str>) -> Result<ApiResponse> {
        client::get(host, url, etag).await
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

    /// GET `url` ([`rest_url`], built once by the caller) as `T`, retried.
    async fn api_json_at<T: DeserializeOwned>(&self, host: &str, url: &str) -> Result<T> {
        retrying(|| client::get(host, url, None)).await?.ok_json(url)
    }

    async fn graphql<T: DeserializeOwned, V: Serialize + Sync + ?Sized>(
        &self,
        host: &str,
        query: &str,
        variables: &V,
    ) -> Result<T> {
        retrying(|| client::graphql(host, query, variables)).await
    }
}

/// `s` lowercased in place: GitHub's enum values are ASCII.
pub(crate) fn lowercase(mut s: String) -> String {
    s.make_ascii_lowercase();
    s
}

/// REST page size: GitHub's maximum `per_page`.
pub(crate) const PAGE: usize = 100;

/// gh's bucket for a check run: pass | fail | pending | skipping | cancel.
/// Takes REST (lowercase) and GraphQL (uppercase) values alike.
pub(crate) fn run_bucket(status: &str, conclusion: Option<&str>) -> &'static str {
    if !status.eq_ignore_ascii_case("completed") {
        return "pending";
    }
    let is = |value: &str| conclusion.is_some_and(|c| c.eq_ignore_ascii_case(value));
    if is("success") {
        "pass"
    } else if is("skipped") || is("neutral") {
        "skipping"
    } else if is("cancelled") {
        "cancel"
    } else if is("stale") {
        "pending"
    } else {
        "fail"
    }
}

/// gh's bucket for a commit status.
pub(crate) fn status_bucket(state: &str) -> &'static str {
    let is = |value: &str| state.eq_ignore_ascii_case(value);
    if is("success") {
        "pass"
    } else if is("pending") || is("expected") {
        "pending"
    } else {
        "fail"
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
    pub bucket: &'static str,
    /// Owning workflow; absent for commit statuses.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow: String,
    /// The check's page, where its logs are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Checks tallied by bucket; buckets with none are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CheckCounts {
    #[serde(skip_serializing_if = "is_zero")]
    pub pass: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub fail: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub pending: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub skipped: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub cancelled: u32,
}

impl CheckCounts {
    pub fn tally(checks: &[PrCheck]) -> Self {
        let mut counts = Self::default();
        for check in checks {
            counts.add(check.bucket);
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

    pub const fn total(&self) -> u32 {
        self.pass + self.fail + self.pending + self.skipped + self.cancelled
    }

    pub const fn conclusion(&self) -> &'static str {
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
