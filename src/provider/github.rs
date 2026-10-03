use crate::error::{CliError, Result};
use crate::executor::{self};
use crate::provider::{BoxFuture, BuildRun, Provider, ProviderExt};
use crate::version::minimum;
use chrono_machines::{AsyncRetryable, ExponentialBackoff, RetryOutcome};
use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::time::Duration;

mod discussion;
pub use discussion::{PrDiscussion, PrNote};
pub(crate) use discussion::{User, clip, login};

const RUN_FIELDS: &str = "databaseId,status,conclusion,headBranch,workflowName,displayTitle,url";

const CHECK_FIELDS: &str = "bucket,name,workflow,link,description";

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
            // Plain `auth status` exits 1 both when logged out and when its API
            // check times out; `--json` tells them apart. Older gh lacks it.
            let args = ["auth", "status", "--active", "--json", "hosts"];
            match executor::execute_success(self.cli(), &args, Duration::from_secs(10)).await {
                Ok(output) => output.json::<AuthReport>(self.cli())?.logged_in(),
                Err(CliError::ExecutionFailed { .. }) => super::cli_authenticated(self.cli()).await,
                Err(e) => Err(e),
            }
        })
    }

    fn ci_runs(&self, limit: usize) -> BoxFuture<'_, Result<Vec<BuildRun>>> {
        Box::pin(async move {
            let limit = limit.to_string();
            let runs: Vec<WorkflowRun> = self
                .execute_json(&["run", "list", "--json", RUN_FIELDS, "--limit", &limit])
                .await?;
            Ok(runs.into_iter().map(Into::into).collect())
        })
    }

    fn ci_run<'a>(&'a self, id: Option<&'a str>) -> BoxFuture<'a, Result<BuildRun>> {
        Box::pin(async move {
            match id {
                Some(id) => {
                    let run: WorkflowRun =
                        self.execute_json(&["run", "view", id, "--json", RUN_FIELDS]).await?;
                    Ok(run.into())
                }
                None => self.ci_runs(1).await?.pop().ok_or_else(|| {
                    CliError::parse_error(self.cli(), "", "no CI runs found for this repository")
                }),
            }
        })
    }
}

impl GitHubProvider {
    /// GET through `gh api`, conditional on `etag` when given. Any HTTP status
    /// comes back as `Ok`; only a failure to get a response is an `Err`.
    pub async fn api_get(&self, host: &str, path: &str, etag: Option<&str>) -> Result<ApiResponse> {
        let condition = etag.map(|etag| format!("If-None-Match: {etag}"));
        let mut args = vec!["api", "-i", "-H", "Accept: application/vnd.github+json"];
        if host != "github.com" {
            args.extend(["--hostname", host]);
        }
        if let Some(condition) = &condition {
            args.extend(["-H", condition.as_str()]);
        }
        args.push(path);

        let output = executor::execute(self.cli(), &args, Duration::from_secs(20)).await?;
        ApiResponse::parse(&output.stdout).ok_or_else(|| {
            CliError::execution_failed(self.cli(), output.code, output.stderr.trim())
        })
    }

    pub async fn pr_checks(&self, pr: Option<u64>) -> Result<Vec<PrCheck>> {
        let pr_str;
        let mut args = vec!["pr", "checks"];
        if let Some(n) = pr {
            pr_str = n.to_string();
            args.push(&pr_str);
        }
        args.extend(["--json", CHECK_FIELDS]);
        self.checks_output(&args, Duration::from_secs(30)).await
    }
    pub async fn pr_checks_watch(
        &self,
        pr: Option<u64>,
        fail_fast: bool,
        timeout: Duration,
        interval: Duration,
    ) -> Result<ChecksWatch> {
        watch_checks(|| self.pr_checks(pr), fail_fast, timeout, interval).await
    }
    /// The PR of the current branch.
    pub async fn current_pr(&self) -> Result<u64> {
        #[derive(Deserialize)]
        struct View {
            number: u64,
        }
        let view: View = self.execute_json(&["pr", "view", "--json", "number"]).await?;
        Ok(view.number)
    }

    pub async fn pr_list(&self, state: &str, author: &str) -> Result<Vec<PrState>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Listed {
            number: u64,
            state: String,
            merged_at: Option<String>,
        }
        let author = if author == "me" { "@me" } else { author };
        let args = [
            "pr",
            "list",
            "--state",
            state,
            "--author",
            author,
            "--limit",
            "30",
            "--json",
            "number,state,mergedAt",
        ];
        let listed: Vec<Listed> = retrying(|| self.execute_json(&args)).await?;
        Ok(listed
            .into_iter()
            .map(|l| PrState {
                pr: l.number,
                state: l.state.to_lowercase(),
                merged_at: l.merged_at,
            })
            .collect())
    }

    async fn api_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = retrying(|| self.api_get("github.com", path, None)).await?;
        if response.status != 200 {
            return Err(CliError::execution_failed(
                self.cli(),
                i32::from(response.status),
                response.body.trim(),
            ));
        }
        response.json()
    }

    async fn checks_output(&self, args: &[&str], timeout: Duration) -> Result<Vec<PrCheck>> {
        let output = executor::execute(self.cli(), args, timeout).await?;

        if output.stdout.trim().is_empty() {
            if output.stderr.contains("no checks reported") {
                return Ok(Vec::new());
            }
            return Err(CliError::execution_failed(self.cli(), output.code, output.stderr));
        }

        output.json(self.cli())
    }
}

/// REST page size: GitHub's maximum `per_page`.
pub(crate) const PAGE: usize = 100;

/// `call`, retried up to three times (~1s doubling, jittered) while it fails
/// on the network. Any other error returns at once.
async fn retrying<T, F, Fut>(call: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let backoff = ExponentialBackoff::new()
        .base_delay_ms(1_000)
        .multiplier(2.0)
        .max_delay_ms(4_000)
        .max_attempts(4)
        .jitter_factor(0.5);
    call.retry_async(backoff)
        .when(|e: &CliError| super::network_failure(&e.to_string()))
        .call_async(|ms| tokio::time::sleep(Duration::from_millis(ms)))
        .await
        .map(RetryOutcome::into_inner)
        .map_err(|e| e.into_cause().expect("a failed retry carries its last error"))
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
/// or the call itself ran out of time.
fn transient(e: &CliError) -> bool {
    matches!(e, CliError::Timeout { .. }) || super::network_failure(&e.to_string())
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrState {
    pub pr: u64,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
}

/// A `gh api -i` response. 304 means the ETag still matches: nothing changed,
/// and GitHub did not count the request against the rate limit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApiResponse {
    pub status: u16,
    pub etag: Option<String>,
    pub body: String,
    pub rate_remaining: Option<u64>,
    /// Epoch seconds when the rate window resets.
    pub rate_reset: Option<u64>,
    pub retry_after: Option<u64>,
}

impl ApiResponse {
    /// Parse `gh api -i` output: status line, headers, blank line, body.
    pub fn parse(raw: &str) -> Option<Self> {
        let (head, body) =
            raw.split_once("\r\n\r\n").or_else(|| raw.split_once("\n\n")).unwrap_or((raw, ""));
        let mut lines = head.lines();
        let status =
            lines.next()?.strip_prefix("HTTP/")?.split_whitespace().nth(1)?.parse().ok()?;

        let mut response = Self { status, body: body.to_string(), ..Self::default() };
        for (name, value) in lines.filter_map(|line| line.split_once(':')) {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "etag" => response.etag = Some(value.to_string()),
                "x-ratelimit-remaining" => response.rate_remaining = value.parse().ok(),
                "x-ratelimit-reset" => response.rate_reset = value.parse().ok(),
                "retry-after" => response.retry_after = value.parse().ok(),
                _ => {}
            }
        }
        Some(response)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.body).map_err(|e| CliError::json("gh", e))
    }
}

#[derive(Debug, Clone)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub name: String,
    pub owner: Owner,
    pub description: Option<String>,
    pub is_private: bool,
    pub default_branch_ref: Option<BranchRef>,
    pub url: String,
    pub ssh_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Owner {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BranchRef {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: Author,
    pub labels: Vec<Label>,
    pub created_at: String,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub author: Author,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: Author,
    pub labels: Vec<Label>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    #[serde(default)]
    pub mergeable: Option<String>,
    #[serde(default)]
    pub is_draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub name: String,
    #[serde(default)]
    pub protected: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub is_prerelease: bool,
    pub created_at: Option<String>,
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

/// gh reports a missing link or description as `""`; read that as absent.
fn non_empty<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.filter(|s| !s.is_empty()))
}

/// One check from `gh pr checks`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PrCheck {
    pub name: String,
    /// gh's classification: pass | fail | pending | skipping | cancel
    pub bucket: String,
    /// Owning workflow; absent for commit statuses.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow: String,
    /// The check's page, where its logs are.
    #[serde(default, deserialize_with = "non_empty", skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, deserialize_with = "non_empty", skip_serializing_if = "Option::is_none")]
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
            match check.bucket.as_str() {
                "pass" => counts.pass += 1,
                "fail" => counts.fail += 1,
                "skipping" => counts.skipped += 1,
                "cancel" => counts.cancelled += 1,
                _ => counts.pending += 1,
            }
        }
        counts
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub database_id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_branch: Option<String>,
    pub workflow_name: Option<String>,
    pub display_title: Option<String>,
    pub url: Option<String>,
}

impl From<WorkflowRun> for BuildRun {
    fn from(run: WorkflowRun) -> Self {
        let status = match run.status.as_str() {
            "completed" => match run.conclusion.as_deref() {
                Some("success") => "success",
                Some("failure") | Some("startup_failure") | Some("timed_out") => "failure",
                Some("cancelled") => "cancelled",
                Some("skipped") => "skipped",
                _ => "completed",
            },
            "in_progress" => "running",
            "queued" | "requested" | "waiting" | "pending" => "queued",
            _ => "unknown",
        };

        BuildRun {
            id: run.database_id.to_string(),
            status: status.to_string(),
            branch: run.head_branch,
            workflow: run.workflow_name,
            title: run.display_title,
            url: run.url,
        }
    }
}

#[cfg(test)]
mod tests;
