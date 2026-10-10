//! GitHub over HTTP: REST and GraphQL through rama, authenticated with the
//! token `gh auth token` hands out (it honours `GH_TOKEN`). Tokens live in
//! memory only, one per host, and are fetched again after a 401.

use std::collections::HashMap;
use std::fmt::{self, Display, Write as _};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use rama::bytes::Bytes;
use rama::http::body::util::BodyExt as _;
use rama::http::header::{ACCEPT, AUTHORIZATION, IF_NONE_MATCH, USER_AGENT};
use rama::http::service::client::HttpClientExt;
use rama::http::{HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, Result};

pub(crate) const API: &str = "github";

const TIMEOUT: Duration = Duration::from_secs(20);

/// `Authorization` values by host, built once from `gh auth token`.
static TOKENS: LazyLock<Mutex<HashMap<String, HeaderValue>>> = LazyLock::new(Mutex::default);

/// A REST or GraphQL response. 304 means the `ETag` still matches: nothing
/// changed, and GitHub did not count the request against the rate limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiResponse {
    pub status: u16,
    pub etag: Option<String>,
    /// As received; decoded straight from the bytes, never copied into a `String`.
    pub body: Bytes,
    pub rate_remaining: Option<u64>,
    /// Epoch seconds when the rate window resets.
    pub rate_reset: Option<u64>,
    pub retry_after: Option<u64>,
}

impl ApiResponse {
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| CliError::json(API, e))
    }

    /// The body as `T` on 200, else an error carrying GitHub's reason.
    pub fn ok_json<T: DeserializeOwned>(&self, what: impl Display) -> Result<T> {
        match self.status {
            200 | 201 => self.json(),
            _ => Err(self.error(what)),
        }
    }

    pub fn error(&self, what: impl Display) -> CliError {
        #[derive(Deserialize)]
        struct Message {
            message: String,
        }
        let reason = serde_json::from_slice::<Message>(&self.body).map(|m| m.message);
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

/// The REST URL of `path` (`repos/o/r/...`, no leading slash) on `host`:
/// api.github.com, or `/api/v3` on a GitHub Enterprise host. Written in one
/// buffer, so callers hand over the path as `format_args!` instead of a
/// `String` of their own.
pub(crate) fn rest_url(host: &str, path: impl Display) -> String {
    let mut url = String::with_capacity(64);
    let written = if host == "github.com" {
        write!(url, "https://api.github.com/{path}")
    } else {
        write!(url, "https://{host}/api/v3/{path}")
    };
    written.expect("writing to a String cannot fail");
    url
}

fn graphql_url(host: &str) -> std::borrow::Cow<'static, str> {
    if host == "github.com" {
        "https://api.github.com/graphql".into()
    } else {
        format!("https://{host}/api/graphql").into()
    }
}

/// `Authorization` for `host` from `gh auth token`, once per host and process.
async fn token(host: &str) -> Result<HeaderValue> {
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
    let token = output.stdout.trim();
    if token.is_empty() {
        return Err(CliError::auth_required("gh"));
    }
    let mut value = HeaderValue::try_from(format!("Bearer {token}"))
        .map_err(|e| CliError::parse_error("gh", "auth token", e.to_string()))?;
    value.set_sensitive(true);
    TOKENS.lock().expect("token cache lock").insert(host.to_owned(), value.clone());
    Ok(value)
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
async fn send<B: Serialize + Sync + ?Sized>(
    host: &str,
    method: &Method,
    url: &str,
    etag: Option<&str>,
    body: Option<&B>,
) -> Result<ApiResponse> {
    let response = send_once(host, method, url, etag, body).await?;
    if response.status != 401 {
        return Ok(response);
    }
    forget_token(host);
    send_once(host, method, url, etag, body).await
}

async fn send_once<B: Serialize + Sync + ?Sized>(
    host: &str,
    method: &Method,
    url: &str,
    etag: Option<&str>,
    body: Option<&B>,
) -> Result<ApiResponse> {
    let token = token(host).await?;
    let client = crate::provider::http_client();
    let mut request = match method {
        Method::Get => client.get(url),
        Method::Post => client.post(url),
        Method::Put => client.put(url),
    }
    .header(AUTHORIZATION, token)
    .header(ACCEPT, HeaderValue::from_static("application/vnd.github+json"))
    .header(HeaderName::from_static("x-github-api-version"), HeaderValue::from_static("2022-11-28"))
    .header(USER_AGENT, HeaderValue::from_static(concat!("skipper/", env!("CARGO_PKG_VERSION"))));
    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    if let Some(body) = body {
        request = request.json(body);
    }

    let response = request
        .send_with_timeout(TIMEOUT)
        .await
        .map_err(|e| crate::provider::http_error(API, e))?;

    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
    let number = |name: &str| header(name).and_then(|v| v.parse().ok());
    let status = response.status().as_u16();
    let etag = header("etag").map(str::to_owned);
    let (rate_remaining, rate_reset, retry_after) =
        (number("x-ratelimit-remaining"), number("x-ratelimit-reset"), number("retry-after"));
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| crate::provider::http_error(API, e))?
        .to_bytes();
    Ok(ApiResponse { status, etag, body, rate_remaining, rate_reset, retry_after })
}

/// GET `url` (see [`rest_url`]), conditional on `etag` when given.
pub(crate) async fn get(host: &str, url: &str, etag: Option<&str>) -> Result<ApiResponse> {
    send::<()>(host, &Method::Get, url, etag, None).await
}

/// PUT `url` with a JSON body. Not retried: a write is not repeated on a
/// guess that it did not land.
pub(crate) async fn put<B: Serialize + Sync + ?Sized>(
    host: &str,
    url: &str,
    body: &B,
) -> Result<ApiResponse> {
    send(host, &Method::Put, url, None, Some(body)).await
}

/// Run a GraphQL `query`; GraphQL reports its failures in `errors` with a 200.
/// `variables` is serialized straight into the request body.
pub(crate) async fn graphql<T: DeserializeOwned, V: Serialize + Sync + ?Sized>(
    host: &str,
    query: &str,
    variables: &V,
) -> Result<T> {
    #[derive(Serialize)]
    struct Request<'a, V: ?Sized> {
        query: &'a str,
        variables: &'a V,
    }
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

    let body = Request { query, variables };
    let response = send(host, &Method::Post, &graphql_url(host), None, Some(&body)).await?;
    if response.status != 200 {
        return Err(response.error("graphql"));
    }
    let envelope: Envelope<T> = response.json()?;
    if !envelope.errors.is_empty() {
        let messages = fmt::from_fn(|f| {
            for (i, e) in envelope.errors.iter().enumerate() {
                if i > 0 {
                    f.write_str("; ")?;
                }
                f.write_str(&e.message)?;
            }
            Ok(())
        });
        return Err(CliError::execution_failed(API, 200, messages.to_string()));
    }
    envelope.data.ok_or_else(|| CliError::parse_error(API, "", "graphql answered without data"))
}
