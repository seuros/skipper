use crate::error::{CliError, Result};
use crate::provider::is_zero;
use rama::http::header::{ACCEPT, AUTHORIZATION};
use rama::http::service::client::HttpClientExt;
use rama::http::{BodyExtractExt, HeaderValue};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Write as _};
use std::path::PathBuf;
use std::time::Duration;

mod issues;

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
        .map(|l| {
            let mut url = l.url;
            url.truncate(url.trim_end_matches('/').len());
            Credentials { name: l.name, url, token: l.token }
        })
        .collect()
}

fn serves(creds: &Credentials, host: &str) -> bool {
    crate::remote::host_of(&creds.url).is_some_and(|h| h.eq_ignore_ascii_case(host))
}

/// The login for `host` among `creds`, the default first.
pub fn match_host<'a>(creds: &'a [Credentials], host: &str) -> Option<&'a Credentials> {
    creds.iter().find(|c| serves(c, host))
}

pub fn credentials_for_host(host: &str) -> Option<Credentials> {
    load_credentials().into_iter().find(|c| serves(c, host))
}

pub fn any_credentials() -> Option<Credentials> {
    load_credentials().into_iter().next()
}

pub struct ForgejoClient {
    creds: Credentials,
    /// `token …`, built once per client; `None` when the token cannot be a
    /// header value.
    auth: Option<HeaderValue>,
    timeout: Duration,
}

impl ForgejoClient {
    pub fn new(creds: Credentials) -> Self {
        let auth = HeaderValue::try_from(format!("token {}", creds.token)).ok().map(|mut auth| {
            auth.set_sensitive(true);
            auth
        });
        Self { creds, auth, timeout: Duration::from_secs(30) }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.creds.url
    }

    pub fn login_name(&self) -> &str {
        &self.creds.name
    }

    /// The API URL of `path` (leading slash), written in one buffer.
    fn url(&self, path: impl Display) -> String {
        let mut url = String::with_capacity(self.creds.url.len() + 64);
        write!(url, "{}/api/v1{path}", self.creds.url).expect("writing to a String cannot fail");
        url
    }

    /// GET `url` ([`Self::url`], built once by the caller) as `T`, retried.
    async fn get_json<T: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        url: &str,
    ) -> Result<T> {
        let auth = self.auth.as_ref().ok_or_else(|| {
            CliError::parse_error("tea", &self.creds.name, "login token is not a valid header")
        })?;
        super::retrying(|| self.get_json_once(url, auth)).await
    }

    async fn get_json_once<T: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        url: &str,
        auth: &HeaderValue,
    ) -> Result<T> {
        let response = super::http_client()
            .get(url)
            .header(AUTHORIZATION, auth.clone())
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .send_with_timeout(self.timeout)
            .await
            .map_err(|e| super::http_error(API, e))?;

        let status = response.status();
        if !status.is_success() {
            // The body says why (a missing token scope, an unknown repo).
            let reason = response.try_into_json::<ApiMessage>().await.map(|m| m.message);
            let mut detail = format!("{status} {url} (tea login {})", self.creds.name);
            if let Some(reason) = reason.ok().filter(|r| !r.is_empty()) {
                detail.push_str(": ");
                detail.push_str(&reason);
            }
            return Err(CliError::execution_failed(API, i32::from(status.as_u16()), detail));
        }

        response
            .try_into_json::<T>()
            .await
            .map_err(|e| CliError::parse_error(API, url, e.to_string()))
    }

    pub async fn whoami(&self) -> Result<User> {
        self.get_json(&self.url("/user")).await
    }

    pub async fn repo(&self, owner: &str, name: &str) -> Result<Repository> {
        self.get_json(&self.url(issues::repo_path(owner, name))).await
    }

    pub async fn repo_search(
        &self,
        query: Option<&str>,
        limit: u32,
        page: u32,
    ) -> Result<SearchResults> {
        let query = std::fmt::from_fn(|f| match query.map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => write!(f, "&q={}", urlencoding::Encoded(q)),
            None => Ok(()),
        });
        let url = self.url(format_args!("/repos/search?limit={limit}&page={}{query}", page.max(1)));
        self.get_json(&url).await
    }
}

#[derive(Deserialize)]
struct ApiMessage {
    #[serde(default)]
    message: String,
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

#[cfg(test)]
mod tests;
