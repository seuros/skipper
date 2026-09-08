use crate::error::{CliError, Result};
use rama::http::BodyExtractExt;
use rama::http::client::EasyHttpWebClient;
use rama::http::service::client::HttpClientExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

const API: &str = "forgejo";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub name: String,
    pub url: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct TeaConfig {
    #[serde(default)]
    logins: Vec<TeaLogin>,
}

#[derive(Debug, Deserialize)]
struct TeaLogin {
    name: String,
    url: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    default: bool,
}

pub fn tea_config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("SKIPPER_TEA_CONFIG") {
        return Some(PathBuf::from(explicit));
    }

    let home = std::env::var("HOME").ok().map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        home.map(|h| h.join("Library/Application Support/tea/config.yml"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|c| c.join("tea/config.yml"))
    }
}

pub fn load_credentials() -> Vec<Credentials> {
    let Some(path) = tea_config_path() else {
        return Vec::new();
    };
    load_credentials_from(&path)
}

pub fn load_credentials_from(path: &std::path::Path) -> Vec<Credentials> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        tracing::debug!(path = ?path, "no tea config; forgejo credentials unavailable");
        return Vec::new();
    };

    let config: TeaConfig = match serde_yaml_ng::from_str(&raw) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(path = ?path, error = %e, "could not parse tea config");
            return Vec::new();
        }
    };

    let mut logins: Vec<TeaLogin> =
        config.logins.into_iter().filter(|l| !l.token.trim().is_empty()).collect();
    logins.sort_by_key(|l| !l.default);

    logins
        .into_iter()
        .map(|l| Credentials {
            name: l.name,
            url: l.url.trim_end_matches('/').to_string(),
            token: l.token,
        })
        .collect()
}

pub fn match_host(creds: Vec<Credentials>, host: &str) -> Option<Credentials> {
    let host = host.to_lowercase();
    creds.into_iter().find(|c| crate::remote::host_of(&c.url).is_some_and(|h| h == host))
}

pub fn credentials_for_host(host: &str) -> Option<Credentials> {
    match_host(load_credentials(), host)
}

pub fn any_credentials() -> Option<Credentials> {
    load_credentials().into_iter().next()
}

pub struct ForgejoClient {
    creds: Credentials,
    timeout: Duration,
}

impl ForgejoClient {
    pub fn new(creds: Credentials) -> Self {
        Self { creds, timeout: Duration::from_secs(30) }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.creds.url
    }

    pub fn login_name(&self) -> &str {
        &self.creds.name
    }

    async fn get_json<T: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
    ) -> Result<T> {
        let url = format!("{}/api/v1{}", self.creds.url, path);
        let client = EasyHttpWebClient::default();

        let response = client
            .get(&url)
            .header("authorization", format!("token {}", self.creds.token))
            .header("accept", "application/json")
            .send_with_timeout(self.timeout)
            .await
            .map_err(|e| CliError::io(API, std::io::Error::other(e.to_string())))?;

        let status = response.status();
        if !status.is_success() {
            return Err(CliError::execution_failed(
                API,
                status.as_u16() as i32,
                format!("{} {}", status, url),
            ));
        }

        response
            .try_into_json::<T>()
            .await
            .map_err(|e| CliError::parse_error(API, &url, e.to_string()))
    }

    pub async fn whoami(&self) -> Result<User> {
        self.get_json("/user").await
    }

    pub async fn repo(&self, owner: &str, name: &str) -> Result<Repository> {
        let path = format!("/repos/{}/{}", urlencoding::encode(owner), urlencoding::encode(name));
        self.get_json(&path).await
    }

    pub async fn repo_search(
        &self,
        query: Option<&str>,
        limit: u32,
        page: u32,
    ) -> Result<SearchResults> {
        let mut path = format!("/repos/search?limit={limit}&page={}", page.max(1));
        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            path.push_str(&format!("&q={}", urlencoding::encode(q)));
        }
        self.get_json(&path).await
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResults {
    #[serde(default)]
    pub data: Vec<Repository>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub login: String,
    #[serde(default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

/// A repository, trimmed to what a model can act on.
///
/// The API also returns html/ssh/clone URLs, mirrors, sizes, and permission
/// blocks. Those are derivable or irrelevant here and only cost context, so
/// they are deliberately dropped; `full_name` plus the host is enough to
/// reconstruct any URL.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Repository {
    pub full_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub open_issues_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[cfg(test)]
mod tests;
