//! REST check runs and commit statuses, shared with the conditional PR watcher.

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::runs::repo_path;
use super::{GitHubProvider, PAGE, PrCheck, rest_url, run_bucket, status_bucket};
use crate::error::Result;
use crate::workspace::ForgeRepo;

#[derive(Debug, Deserialize)]
pub(crate) struct CheckRuns {
    pub check_runs: Vec<CheckRun>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub html_url: Option<String>,
    pub output: Option<CheckOutput>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CheckOutput {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CombinedStatus {
    pub statuses: Vec<CommitStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CommitStatus {
    pub context: String,
    pub state: String,
    pub target_url: Option<String>,
    pub description: Option<String>,
}

impl From<CheckRun> for PrCheck {
    fn from(run: CheckRun) -> Self {
        Self {
            bucket: run_bucket(&run.status, run.conclusion.as_deref()),
            name: run.name,
            workflow: String::new(),
            link: run.html_url,
            description: run.output.and_then(|o| o.title),
        }
    }
}

impl From<CommitStatus> for PrCheck {
    fn from(status: CommitStatus) -> Self {
        Self {
            bucket: status_bucket(&status.state),
            name: status.context,
            workflow: String::new(),
            link: status.target_url,
            description: status.description,
        }
    }
}

impl GitHubProvider {
    /// Read both sources on the same immutable commit. GitHub's combined
    /// status says "pending" even for zero statuses: count actual entries.
    pub(super) async fn commit_checks(&self, repo: &ForgeRepo, sha: &str) -> Result<Vec<PrCheck>> {
        let (mut checks, statuses) = tokio::try_join!(
            // The check-runs endpoint defaults to filter=latest, and the
            // combined status endpoint returns the latest state per context.
            self.check_pages(repo, sha, "check-runs", |p: CheckRuns| p.check_runs),
            self.check_pages(repo, sha, "status", |p: CombinedStatus| p.statuses),
        )?;
        checks.extend(statuses);
        Ok(checks)
    }

    async fn check_pages<P: DeserializeOwned, T: Into<PrCheck>>(
        &self,
        repo: &ForgeRepo,
        sha: &str,
        endpoint: &str,
        entries: impl Fn(P) -> Vec<T>,
    ) -> Result<Vec<PrCheck>> {
        let base = repo_path(repo);
        let mut checks = Vec::new();
        for page in 1.. {
            let url = rest_url(
                &repo.host,
                format_args!("{base}/commits/{sha}/{endpoint}?per_page={PAGE}&page={page}"),
            );
            let batch = entries(self.api_json_at(&repo.host, &url).await?);
            let full = batch.len() == PAGE;
            checks.extend(batch.into_iter().map(Into::into));
            if !full {
                break;
            }
        }
        Ok(checks)
    }
}
