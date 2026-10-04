//! Actions workflow runs and the account's repositories, over REST.

use serde::{Deserialize, Serialize};

use super::GitHubProvider;
use crate::error::Result;
use crate::provider::BuildRun;
use crate::workspace::ForgeRepo;

impl GitHubProvider {
    /// The latest `limit` workflow runs of `repo`, on `branch` when given.
    pub async fn runs(
        &self,
        repo: &ForgeRepo,
        branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<BuildRun>> {
        #[derive(Deserialize)]
        struct Runs {
            workflow_runs: Vec<WorkflowRun>,
        }
        let mut path =
            format!("{}/actions/runs?per_page={}", repo_path(repo), limit.clamp(1, super::PAGE));
        if let Some(branch) = branch {
            path.push_str(&format!("&branch={}", urlencoding::encode(branch)));
        }
        let runs: Runs = self.api_json_at(&repo.host, &path).await?;
        Ok(runs.workflow_runs.into_iter().map(Into::into).collect())
    }

    pub async fn run(&self, repo: &ForgeRepo, id: &str) -> Result<BuildRun> {
        let path = format!("{}/actions/runs/{}", repo_path(repo), urlencoding::encode(id));
        let run: WorkflowRun = self.api_json_at(&repo.host, &path).await?;
        Ok(run.into())
    }

    /// Repositories the account owns on `host`, most recently pushed first.
    pub async fn owned_repos(&self, host: &str, limit: usize) -> Result<Vec<OwnedRepo>> {
        #[derive(Deserialize)]
        struct Repo {
            name: String,
            owner: super::User,
            description: Option<String>,
        }
        let path = format!(
            "user/repos?affiliation=owner&sort=pushed&per_page={}",
            limit.clamp(1, super::PAGE)
        );
        let repos: Vec<Repo> = self.api_json_at(host, &path).await?;
        Ok(repos
            .into_iter()
            .map(|r| OwnedRepo { name: r.name, owner: r.owner.login, description: r.description })
            .collect())
    }
}

fn repo_path(repo: &ForgeRepo) -> String {
    format!("repos/{}/{}", urlencoding::encode(&repo.owner), urlencoding::encode(&repo.name))
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnedRepo {
    pub name: String,
    pub owner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A workflow run as the REST API reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    #[serde(default)]
    pub status: String,
    pub conclusion: Option<String>,
    pub head_branch: Option<String>,
    /// The workflow's name.
    pub name: Option<String>,
    pub display_title: Option<String>,
    pub html_url: Option<String>,
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
            id: run.id.to_string(),
            status: status.to_string(),
            branch: run.head_branch,
            workflow: run.name,
            title: run.display_title,
            url: run.html_url,
        }
    }
}
