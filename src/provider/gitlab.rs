use crate::error::Result;
use crate::provider::{BoxFuture, BuildRun, Provider, ProviderExt};
use crate::version::minimum;
use semver::Version;
use serde::Deserialize;

pub struct GitLabProvider {
    min_version: Version,
}

impl GitLabProvider {
    pub fn new() -> Self {
        Self { min_version: minimum::gitlab() }
    }
}

impl Default for GitLabProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for GitLabProvider {
    fn name(&self) -> &'static str {
        "gitlab"
    }

    fn cli(&self) -> &'static str {
        "glab"
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
            let pipelines: Vec<Pipeline> = self
                .execute_json(&["ci", "list", "--output", "json", "--per-page", &limit])
                .await?;
            Ok(pipelines.into_iter().map(Into::into).collect())
        })
    }

    fn ci_run<'a>(&'a self, id: Option<&'a str>) -> BoxFuture<'a, Result<BuildRun>> {
        Box::pin(async move {
            let pipeline: Pipeline = match id {
                Some(id) => {
                    self.execute_json(&["ci", "get", "--pipeline-id", id, "--output", "json"])
                        .await?
                }
                None => self.execute_json(&["ci", "get", "--output", "json"]).await?,
            };
            Ok(pipeline.into())
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub path_with_namespace: String,
    pub description: Option<String>,
    pub visibility: String,
    pub default_branch: Option<String>,
    pub web_url: String,
    pub ssh_url_to_repo: Option<String>,
    pub http_url_to_repo: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub author: Author,
    #[serde(default)]
    pub labels: Vec<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub username: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub author: Author,
    #[serde(default)]
    pub labels: Vec<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub merge_status: Option<String>,
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub name: String,
    #[serde(default)]
    pub protected: bool,
    pub default: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub created_at: Option<String>,
    pub released_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    pub sha: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub web_url: Option<String>,
}

impl From<Pipeline> for BuildRun {
    fn from(pipeline: Pipeline) -> Self {
        let status = match pipeline.status.as_str() {
            "success" => "success",
            "failed" => "failure",
            "canceled" | "canceling" => "cancelled",
            "skipped" => "skipped",
            "running" => "running",
            "created"
            | "pending"
            | "waiting_for_resource"
            | "preparing"
            | "scheduled"
            | "manual" => "queued",
            _ => "unknown",
        };

        BuildRun {
            id: pipeline.id.to_string(),
            status: status.to_string(),
            branch: pipeline.ref_name,
            workflow: None,
            title: None,
            url: pipeline.web_url,
        }
    }
}

#[cfg(test)]
mod tests;
