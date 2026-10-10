use crate::error::Result;
use crate::provider::{BoxFuture, BuildRun, CiTarget, Provider, ProviderExt};
use crate::version::minimum;
use semver::Version;
use serde::Deserialize;

pub struct GitLabProvider {
    min_version: Version,
}

impl GitLabProvider {
    pub const fn new() -> Self {
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

    fn check_auth(&self) -> BoxFuture<'_, crate::error::Result<bool>> {
        Box::pin(super::cli_authenticated(self.cli()))
    }

    fn ci_runs<'a>(
        &'a self,
        _target: &'a CiTarget,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move {
            let limit = limit.to_string();
            let pipelines: Vec<Pipeline> = self
                .execute_json(&["ci", "list", "--output", "json", "--per-page", &limit])
                .await?;
            Ok(pipelines.into_iter().map(Into::into).collect())
        })
    }

    fn ci_run<'a>(
        &'a self,
        _env: &'a crate::environment::SkipperEnvironment,
        _target: &'a CiTarget,
        id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<BuildRun>> {
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
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
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

        Self {
            id: pipeline.id.to_string(),
            status,
            branch: pipeline.ref_name,
            workflow: None,
            title: None,
            url: pipeline.web_url,
        }
    }
}

#[cfg(test)]
mod tests;
