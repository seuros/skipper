//! GitHub over HTTP: REST and GraphQL through rama, authenticated with the
//! token `gh auth token` hands out (it honours `GH_TOKEN`). Tokens live in
//! memory only, one per host, and are fetched again after a 401.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use rama::http::BodyExtractExt;
use rama::http::client::EasyHttpWebClient;
use rama::http::service::client::HttpClientExt;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{CliError, Result};

pub(crate) const API: &str = "github";

const TIMEOUT: Duration = Duration::from_secs(20);

static TOKENS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

/// A REST or GraphQL response. 304 means the ETag still matches: nothing
/// changed, and GitHub did not count the request against the rate limit.
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
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.body).map_err(|e| CliError::json(API, e))
    }

    /// The body as `T` on 200, else an error carrying GitHub's reason.
    pub fn ok_json<T: DeserializeOwned>(&self, what: &str) -> Result<T> {
        match self.status {
            200 | 201 => self.json(),
            _ => Err(self.error(what)),
        }
    }

    pub fn error(&self, what: &str) -> CliError {
        #[derive(Deserialize)]
        struct Message {
            message: String,
        }
        let reason = serde_json::from_str::<Message>(&self.body).map(|m| m.message);
        match (self.status, reason) {
            (401, _) => CliError::auth_required("gh"),
            (status, Ok(reason)) => {
                CliError::execution_failed(API, i32::from(status), format!("{what}: {reason}"))
            }
            (status, Err(_)) => {
                CliError::execution_failed(API, i32::from(status), format!("{what}: HTTP {status}"))
            }
        }
    }
}

/// REST root: api.github.com, or `/api/v3` on a GitHub Enterprise host.
fn rest_url(host: &str, path: &str) -> String {
    let path = path.trim_start_matches('/');
    if host == "github.com" {
        format!("https://api.github.com/{path}")
    } else {
        format!("https://{host}/api/v3/{path}")
    }
}

fn graphql_url(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com/graphql".to_string()
    } else {
        format!("https://{host}/api/graphql")
    }
}

/// `gh auth token` for `host`, once per host and process.
async fn token(host: &str) -> Result<String> {
    if let Some(token) = TOKENS.lock().expect("token cache lock").get(host) {
        return Ok(token.clone());
    }
    let args = ["auth", "token", "--hostname", host];
    let output = crate::executor::execute_success("gh", &args, Duration::from_secs(10))
        .await
        .map_err(|e| match e {
            CliError::ExecutionFailed { .. } => CliError::auth_required("gh"),
            e => e,
        })?;
    let token = output.stdout.trim().to_string();
    if token.is_empty() {
        return Err(CliError::auth_required("gh"));
    }
    TOKENS.lock().expect("token cache lock").insert(host.to_string(), token.clone());
    Ok(token)
}

fn forget_token(host: &str) {
    TOKENS.lock().expect("token cache lock").remove(host);
}

enum Method {
    Get,
    Post,
    Put,
}

/// One request. Any HTTP status comes back as `Ok`; only a failure to get a
/// response is an `Err`. A 401 drops the cached token and tries once more, in
/// case it was revoked or rotated since.
async fn send(
    host: &str,
    method: &Method,
    url: &str,
    etag: Option<&str>,
    body: Option<&Value>,
) -> Result<ApiResponse> {
    let response = send_once(host, method, url, etag, body).await?;
    if response.status != 401 {
        return Ok(response);
    }
    forget_token(host);
    send_once(host, method, url, etag, body).await
}

async fn send_once(
    host: &str,
    method: &Method,
    url: &str,
    etag: Option<&str>,
    body: Option<&Value>,
) -> Result<ApiResponse> {
    let token = token(host).await?;
    let client = EasyHttpWebClient::default();
    let mut request = match method {
        Method::Get => client.get(url),
        Method::Post => client.post(url),
        Method::Put => client.put(url),
    }
    .header("authorization", format!("Bearer {token}"))
    .header("accept", "application/vnd.github+json")
    .header("x-github-api-version", "2022-11-28")
    .header("user-agent", concat!("skipper/", env!("CARGO_PKG_VERSION")));
    if let Some(etag) = etag {
        request = request.header("if-none-match", etag);
    }
    if let Some(body) = body {
        request = request.json(body);
    }

    let response = request
        .send_with_timeout(TIMEOUT)
        .await
        .map_err(|e| CliError::io(API, std::io::Error::other(e.to_string())))?;

    let header =
        |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut parsed = ApiResponse {
        status: response.status().as_u16(),
        etag: header("etag"),
        rate_remaining: header("x-ratelimit-remaining").and_then(|v| v.parse().ok()),
        rate_reset: header("x-ratelimit-reset").and_then(|v| v.parse().ok()),
        retry_after: header("retry-after").and_then(|v| v.parse().ok()),
        body: String::new(),
    };
    parsed.body = response
        .try_into_string()
        .await
        .map_err(|e| CliError::io(API, std::io::Error::other(e.to_string())))?;
    Ok(parsed)
}

/// GET a REST `path` (`repos/o/r/...`), conditional on `etag` when given.
pub(crate) async fn get(host: &str, path: &str, etag: Option<&str>) -> Result<ApiResponse> {
    send(host, &Method::Get, &rest_url(host, path), etag, None).await
}

/// PUT a REST `path` with a JSON body. Not retried: a write is not repeated
/// on a guess that it did not land.
pub(crate) async fn put(host: &str, path: &str, body: &Value) -> Result<ApiResponse> {
    send(host, &Method::Put, &rest_url(host, path), None, Some(body)).await
}

/// Run a GraphQL `query`; GraphQL reports its failures in `errors` with a 200.
pub(crate) async fn graphql<T: DeserializeOwned>(
    host: &str,
    query: &str,
    variables: Value,
) -> Result<T> {
    #[derive(Deserialize)]
    struct Envelope<T> {
        data: Option<T>,
        #[serde(default)]
        errors: Vec<GraphqlError>,
    }
    #[derive(Deserialize)]
    struct GraphqlError {
        message: String,
    }

    let body = serde_json::json!({ "query": query, "variables": variables });
    let response = send(host, &Method::Post, &graphql_url(host), None, Some(&body)).await?;
    if response.status != 200 {
        return Err(response.error("graphql"));
    }
    let envelope: Envelope<T> = response.json()?;
    if !envelope.errors.is_empty() {
        let messages: Vec<String> = envelope.errors.into_iter().map(|e| e.message).collect();
        return Err(CliError::execution_failed(API, 200, messages.join("; ")));
    }
    envelope.data.ok_or_else(|| CliError::parse_error(API, "", "graphql answered without data"))
}
