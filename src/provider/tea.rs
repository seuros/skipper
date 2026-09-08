use crate::provider::{BoxFuture, Provider};
use crate::version::minimum;
use semver::Version;
use serde::Deserialize;

pub const REPO_FIELDS: &str = "id,owner,name,type,description,url,ssh,permission";

pub struct TeaProvider {
    min_version: Version,
}

impl TeaProvider {
    pub fn new() -> Self {
        Self { min_version: minimum::tea() }
    }
}

impl Default for TeaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for TeaProvider {
    fn name(&self) -> &'static str {
        "tea"
    }

    fn cli(&self) -> &'static str {
        "tea"
    }

    fn min_version(&self) -> Version {
        self.min_version.clone()
    }

    fn check_auth(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let Some(creds) = super::forgejo::any_credentials() else {
                tracing::debug!("no tea login with a token; forge tools stay hidden");
                return false;
            };

            let client = super::forgejo::ForgejoClient::new(creds)
                .with_timeout(std::time::Duration::from_secs(10));

            match client.whoami().await {
                Ok(user) => {
                    tracing::info!(
                        login = client.login_name(),
                        url = client.base_url(),
                        user = user.login,
                        "forgejo token validated"
                    );
                    true
                }
                Err(e) => {
                    tracing::warn!(
                        login = client.login_name(),
                        error = %e,
                        "forgejo token rejected"
                    );
                    false
                }
            }
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Login {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub ssh_host: Option<String>,
    pub user: String,
    #[serde(default)]
    pub default: Option<String>,
}

impl Login {
    pub fn is_default(&self) -> bool {
        self.default.as_deref() == Some("true")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    #[serde(default)]
    pub id: Option<String>,
    pub owner: String,
    pub name: String,
    #[serde(rename = "type", default)]
    pub repo_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub ssh: Option<String>,
    #[serde(default)]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub index: String,
    pub title: String,
    pub state: String,
    pub author: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub labels: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl Issue {
    pub fn index_u64(&self) -> Option<u64> {
        self.index.parse().ok()
    }

    pub fn label_names(&self) -> Vec<String> {
        split_labels(self.labels.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssueDetail {
    pub id: u64,
    pub index: u64,
    pub title: String,
    pub state: String,
    pub user: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "closedAt", default)]
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub index: String,
    pub title: String,
    pub state: String,
    pub author: String,
    #[serde(default)]
    pub body: Option<String>,
    pub base: String,
    pub head: String,
    #[serde(default)]
    pub mergeable: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub labels: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl PullRequest {
    pub fn index_u64(&self) -> Option<u64> {
        self.index.parse().ok()
    }

    pub fn is_mergeable(&self) -> Option<bool> {
        self.mergeable.as_deref().map(|m| m == "true")
    }

    pub fn label_names(&self) -> Vec<String> {
        split_labels(self.labels.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestDetail {
    pub id: u64,
    pub index: u64,
    pub title: String,
    pub state: String,
    pub user: String,
    #[serde(default)]
    pub body: Option<String>,
    pub base: String,
    pub head: String,
    #[serde(rename = "headSha", default)]
    pub head_sha: Option<String>,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(rename = "hasMerged", default)]
    pub has_merged: bool,
    #[serde(rename = "mergedAt", default)]
    pub merged_at: Option<String>,
    #[serde(rename = "closedAt", default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub name: String,
    #[serde(default)]
    pub protected: Option<String>,
}

impl Branch {
    pub fn is_protected(&self) -> bool {
        self.protected.as_deref() == Some("true")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    #[serde(rename = "tag-_name", alias = "tag_name", alias = "tag-name")]
    pub tag_name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "published _at", alias = "published_at", default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(rename = "tar/_zip url", alias = "archive_urls", default)]
    pub archive_urls: Option<String>,
}

fn split_labels(labels: Option<&str>) -> Vec<String> {
    labels
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests;
