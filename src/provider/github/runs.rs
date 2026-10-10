//! Actions workflow runs and the account's repositories, over REST.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{GitHubProvider, rest_url};
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
        let branch = fmt::from_fn(|f| match branch {
            Some(branch) => write!(f, "&branch={}", urlencoding::encode(branch)),
            None => Ok(()),
        });
        let url = rest_url(
            &repo.host,
            format_args!(
                "{}/actions/runs?per_page={}{branch}",
                repo_path(repo),
                limit.clamp(1, super::PAGE)
            ),
        );
        let runs: Runs = self.api_json_at(&repo.host, &url).await?;
        Ok(runs.workflow_runs.into_iter().map(Into::into).collect())
    }

    /// Every workflow run of commit `sha`.
    pub async fn commit_runs(&self, repo: &ForgeRepo, sha: &str) -> Result<Vec<BuildRun>> {
        #[derive(Deserialize)]
        struct Runs {
            workflow_runs: Vec<WorkflowRun>,
        }
        let url = rest_url(
            &repo.host,
            format_args!(
                "{}/actions/runs?head_sha={}&per_page={}",
                repo_path(repo),
                urlencoding::encode(sha),
                super::PAGE
            ),
        );
        let runs: Runs = self.api_json_at(&repo.host, &url).await?;
        Ok(runs.workflow_runs.into_iter().map(Into::into).collect())
    }

    pub async fn run(&self, repo: &ForgeRepo, id: &str) -> Result<BuildRun> {
        let url = rest_url(
            &repo.host,
            format_args!("{}/actions/runs/{}", repo_path(repo), urlencoding::encode(id)),
        );
        let run: WorkflowRun = self.api_json_at(&repo.host, &url).await?;
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
        let url = rest_url(
            host,
            format_args!(
                "user/repos?affiliation=owner&sort=pushed&per_page={}",
                limit.clamp(1, super::PAGE)
            ),
        );
        let repos: Vec<Repo> = self.api_json_at(host, &url).await?;
        Ok(repos
            .into_iter()
            .map(|r| OwnedRepo { name: r.name, owner: r.owner.login, description: r.description })
            .collect())
    }
}

/// `repos/{owner}/{name}`, written straight into the URL being built.
pub(super) fn repo_path(repo: &ForgeRepo) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        write!(f, "repos/{}/{}", urlencoding::encode(&repo.owner), urlencoding::encode(&repo.name))
    })
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
                Some("failure" | "startup_failure" | "timed_out") => "failure",
                Some("cancelled") => "cancelled",
                Some("skipped") => "skipped",
                _ => "completed",
            },
            "in_progress" => "running",
            "queued" | "requested" | "waiting" | "pending" => "queued",
            _ => "unknown",
        };

        Self {
            id: run.id.to_string(),
            status,
            branch: run.head_branch,
            workflow: run.name,
            title: run.display_title,
            url: run.html_url,
        }
    }
}
