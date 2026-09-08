use crate::error::{CliError, Result};
use crate::executor::{self};
use crate::provider::{BoxFuture, BuildRun, Provider, ProviderExt};
use crate::version::minimum;
use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const RUN_FIELDS: &str = "databaseId,status,conclusion,headBranch,workflowName,displayTitle,url";

const CHECK_FIELDS: &str =
    "bucket,name,workflow,state,startedAt,completedAt,link,description,event";

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

    fn check_auth(&self) -> BoxFuture<'_, bool> {
        Box::pin(super::cli_authenticated(self.cli()))
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
    ) -> Result<Vec<PrCheck>> {
        let start = tokio::time::Instant::now();
        let deadline = start + timeout;
        let registration_grace = Duration::from_secs(120).min(timeout);

        loop {
            let checks = self.pr_checks(pr).await?;
            let counts = CheckCounts::tally(&checks);
            let now = tokio::time::Instant::now();

            if checks.is_empty() {
                if now.duration_since(start) >= registration_grace {
                    return Ok(checks);
                }
            } else {
                if counts.pending == 0 {
                    return Ok(checks);
                }
                if fail_fast && counts.fail > 0 {
                    return Ok(checks);
                }
            }

            if now >= deadline {
                return Err(CliError::timeout(self.cli(), timeout));
            }

            tokio::time::sleep(interval.min(deadline - now)).await;
        }
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

fn non_zero_timestamp<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.filter(|s| !s.is_empty() && !s.starts_with("0001-01-01")))
}

/// One check from `gh pr checks`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    /// gh's classification: pass | fail | pending | skipping | cancel
    pub bucket: String,
    /// Owning workflow; empty for commit statuses.
    #[serde(default)]
    pub workflow: String,
    pub state: String,
    /// gh emits a zero timestamp for checks that have not started or
    /// finished; those read as `null` rather than the year 1.
    #[serde(default, deserialize_with = "non_zero_timestamp")]
    pub started_at: Option<String>,
    #[serde(default, deserialize_with = "non_zero_timestamp")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub event: Option<String>,
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
