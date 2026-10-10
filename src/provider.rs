#[cfg(feature = "github")]
pub mod github;

#[cfg(feature = "tea")]
pub mod tea;

#[cfg(feature = "gitlab")]
pub mod gitlab;

#[cfg(any(feature = "github", feature = "tea"))]
pub mod issues;

#[cfg(any(feature = "github", feature = "tea"))]
pub(crate) mod text;

use crate::error::{CliError, Result};
use crate::executor;
use crate::version;
use chrono_machines::{BackoffStrategy, ExponentialBackoff};
use semver::Version;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub enum ProviderStatus {
    Available {
        version: Version,
    },
    NotInstalled,
    VersionTooLow {
        found: Version,
        required: Version,
    },
    AuthRequired,
    /// A probe timed out or hit a network error; worth retrying.
    Unreachable,
}

impl ProviderStatus {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub const fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable)
    }

    pub const fn version(&self) -> Option<&Version> {
        match self {
            Self::Available { version } => Some(version),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct BuildRun {
    pub id: String,
    /// success | failure | cancelled | skipped | completed | running | queued | unknown
    pub status: &'static str,
    pub branch: Option<String>,
    pub workflow: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
}

impl BuildRun {
    pub fn is_terminal(&self) -> bool {
        is_terminal_status(self.status)
    }
}

pub fn is_terminal_status(status: &str) -> bool {
    matches!(status, "success" | "failure" | "cancelled" | "skipped" | "completed")
}

/// One verdict for a set of runs, with the precedence PR checks use:
/// failure, then cancelled, then pending, else success. `no_runs` while none
/// has registered.
pub fn conclusion_of(runs: &[BuildRun]) -> &'static str {
    if runs.is_empty() {
        "no_runs"
    } else if runs.iter().any(|r| r.status == "failure") {
        "failure"
    } else if runs.iter().any(|r| r.status == "cancelled") {
        "cancelled"
    } else if runs.iter().any(|r| !r.is_terminal()) {
        "pending"
    } else {
        "success"
    }
}

/// `auth status` exits non-zero for a failed network round-trip too; these
/// markers separate that from a missing or rejected login.
#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
const NETWORK_FAILURES: &[&str] = &[
    "timeout",
    "timed out",
    "connection reset",
    "connection refused",
    "dial tcp",
    "no such host",
    "no route to host",
    "network is unreachable",
    "tls handshake",
    "unexpected eof",
    "bad gateway",
    "service unavailable",
];

#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
fn network_failure(output: &str) -> bool {
    NETWORK_FAILURES.iter().any(|marker| crate::ascii::contains_ignore_case(output, marker))
}

/// A forge HTTP request that got no response.
#[cfg(any(feature = "github", feature = "tea"))]
pub(crate) fn http_error(api: &str, e: impl std::fmt::Display) -> CliError {
    CliError::io(api, std::io::Error::other(e.to_string()))
}

/// The one HTTP client of the process: its connection pool and TLS config
/// are built once, so forge calls reuse connections instead of handshaking
/// per request.
#[cfg(any(feature = "github", feature = "tea"))]
pub(crate) fn http_client() -> &'static rama::http::client::DefaultHttpWebClient {
    static CLIENT: std::sync::LazyLock<rama::http::client::DefaultHttpWebClient> =
        std::sync::LazyLock::new(Default::default);
    &CLIENT
}

/// `skip_serializing_if` for counters.
#[cfg(any(feature = "github", feature = "tea"))]
#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde passes the field by reference")]
pub(crate) const fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[cfg(any(feature = "github", feature = "gitlab"))]
async fn cli_authenticated(cli: &str) -> Result<bool> {
    let output = executor::execute(cli, &["auth", "status"], Duration::from_secs(5)).await?;
    if output.success() {
        return Ok(true);
    }
    if network_failure(&output.stdout) || network_failure(&output.stderr) {
        return Err(CliError::execution_failed(cli, output.code, output.stderr));
    }
    Ok(false)
}

/// Attempts `retrying` makes before giving up.
#[cfg(any(feature = "github", feature = "tea"))]
pub(crate) const RETRY_ATTEMPTS: u8 = 4;

/// `call`, retried up to three times (~1s doubling, jittered) while it fails
/// in a way a repeat can fix. Any other error returns at once. The one retry
/// layer for forge HTTP calls: resources answer its exhaustion with
/// `RetryExhausted`, which mcp-host does not retry again.
#[cfg(any(feature = "github", feature = "tea"))]
pub(crate) async fn retrying<T, F, Fut>(call: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    use chrono_machines::{AsyncRetryable, RetryOutcome};

    let backoff = ExponentialBackoff::new()
        .base_delay_ms(1_000)
        .multiplier(2.0)
        .max_delay_ms(4_000)
        .max_attempts(RETRY_ATTEMPTS)
        .jitter_factor(0.5);
    call.retry_async(backoff)
        .when(retryable)
        .call_async(|ms| tokio::time::sleep(Duration::from_millis(ms)))
        .await
        .map(RetryOutcome::into_inner)
        .map_err(|e| e.into_cause().expect("a failed retry carries its last error"))
}

/// The network dropped, or the forge answered 5xx: a repeat may get through.
#[cfg(any(feature = "github", feature = "tea"))]
pub(crate) fn retryable(e: &CliError) -> bool {
    matches!(e, CliError::Io { .. } | CliError::ExecutionFailed { code: 500..=599, .. })
        || network_failure(&e.to_string())
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;

    fn cli(&self) -> &'static str;

    fn min_version(&self) -> Version;

    fn detect(&self) -> BoxFuture<'_, ProviderStatus> {
        Box::pin(async move {
            let version_output = match executor::get_version_output(self.cli()).await {
                Ok(output) => output,
                Err(e @ (CliError::Timeout { .. } | CliError::Io { .. })) => {
                    tracing::warn!(cli = self.cli(), error = %e, "version probe failed");
                    return ProviderStatus::Unreachable;
                }
                Err(_) => return ProviderStatus::NotInstalled,
            };

            let Ok(version) = version::parse_version(&version_output, self.cli()) else {
                return ProviderStatus::NotInstalled;
            };

            if version < self.min_version() {
                return ProviderStatus::VersionTooLow {
                    found: version,
                    required: self.min_version(),
                };
            }

            match self.check_auth().await {
                Ok(true) => ProviderStatus::Available { version },
                Ok(false) => ProviderStatus::AuthRequired,
                Err(e) => {
                    tracing::warn!(cli = self.cli(), error = %e, "auth probe failed");
                    ProviderStatus::Unreachable
                }
            }
        })
    }

    /// `Ok(false)`: definitely not logged in. `Err`: the probe itself failed
    /// (timeout, network), so the answer is unknown.
    fn check_auth(&self) -> BoxFuture<'_, Result<bool>>;

    fn execute<'a>(&'a self, args: &'a [&'a str]) -> BoxFuture<'a, Result<executor::Output>> {
        Box::pin(executor::execute_success(self.cli(), args, Duration::from_secs(30)))
    }

    fn execute_with_timeout<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: Duration,
    ) -> BoxFuture<'a, Result<executor::Output>> {
        Box::pin(executor::execute_success(self.cli(), args, timeout))
    }

    /// Recent CI runs of the workspace's repo on this forge.
    fn ci_runs<'a>(
        &'a self,
        _env: &'a crate::environment::SkipperEnvironment,
        _limit: usize,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move { Err(CliError::unsupported(self.cli(), "CI run listing")) })
    }

    /// Every run of commit `sha` (full id), newest first.
    fn ci_runs_for_commit<'a>(
        &'a self,
        _env: &'a crate::environment::SkipperEnvironment,
        _sha: &'a str,
    ) -> BoxFuture<'a, Result<Vec<BuildRun>>> {
        Box::pin(async move { Err(CliError::unsupported(self.cli(), "CI runs by commit")) })
    }

    /// Run `id`, or the latest run of the current branch.
    fn ci_run<'a>(
        &'a self,
        _env: &'a crate::environment::SkipperEnvironment,
        _id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<BuildRun>> {
        Box::pin(async move { Err(CliError::unsupported(self.cli(), "CI run status")) })
    }
}

pub trait ProviderExt: Provider {
    fn execute_json<T: serde::de::DeserializeOwned>(
        &self,
        args: &[&str],
    ) -> impl std::future::Future<Output = Result<T>> + Send;
}

impl<P: Provider> ProviderExt for P {
    async fn execute_json<T: serde::de::DeserializeOwned>(&self, args: &[&str]) -> Result<T> {
        let output = self.execute(args).await?;
        output.json(self.cli())
    }
}

/// Re-probe delays for unreachable providers: ~5s doubling to a 60s cap,
/// seven retries (a few minutes) before giving up.
fn reprobe_backoff() -> ExponentialBackoff {
    ExponentialBackoff::new()
        .base_delay_ms(5_000)
        .multiplier(2.0)
        .max_delay_ms(60_000)
        .max_attempts(8)
        .jitter_factor(0.5)
}

pub struct Registry {
    providers: HashMap<&'static str, Arc<dyn Provider>>,
    status: RwLock<HashMap<&'static str, ProviderStatus>>,
}

impl Registry {
    pub fn new() -> Self {
        Self { providers: HashMap::new(), status: RwLock::default() }
    }

    /// Every forge built in, less those `disabled` names.
    pub fn with_defaults(disabled: impl Fn(&str) -> bool) -> Self {
        let mut registry = Self::new();
        let providers: [Option<Box<dyn Provider>>; 3] = [
            cfg_select! {
                feature = "github" => { Some(Box::new(github::GitHubProvider::new())) }
                _ => { None }
            },
            cfg_select! {
                feature = "tea" => { Some(Box::new(tea::TeaProvider::new())) }
                _ => { None }
            },
            cfg_select! {
                feature = "gitlab" => { Some(Box::new(gitlab::GitLabProvider::new())) }
                _ => { None }
            },
        ];
        for provider in providers.into_iter().flatten() {
            if disabled(provider.name()) {
                tracing::info!(provider = provider.name(), "disabled in config");
            } else {
                registry.register(provider);
            }
        }
        registry
    }

    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.insert(provider.name(), Arc::from(provider));
    }

    pub async fn detect_all(&self) -> Vec<(&'static str, ProviderStatus)> {
        let names: Vec<&'static str> = self.providers.keys().copied().collect();
        self.detect(&names).await
    }

    /// Probe `names` concurrently and record their status.
    pub async fn detect(&self, names: &[&'static str]) -> Vec<(&'static str, ProviderStatus)> {
        let mut probes = tokio::task::JoinSet::new();
        for &name in names {
            if let Some(provider) = self.get_arc(name) {
                probes.spawn(async move { (name, provider.detect().await) });
            }
        }

        let mut results = Vec::new();
        while let Some(joined) = probes.join_next().await {
            let (name, status) = match joined {
                Ok(probe) => probe,
                Err(e) => {
                    tracing::error!(error = %e, "provider probe task failed");
                    continue;
                }
            };
            tracing::info!(provider = name, status = ?status, "provider detection complete");
            self.status_mut().insert(name, status.clone());
            results.push((name, status));
        }

        results
    }

    /// Providers whose last probe failed transiently.
    pub fn unreachable(&self) -> Vec<&'static str> {
        self.status_ref().iter().filter(|(_, s)| s.is_unreachable()).map(|(n, _)| *n).collect()
    }

    /// Re-probe unreachable providers with backoff until none remain or the
    /// attempts run out. `on_available` receives each batch that came online.
    pub async fn retry_unreachable(&self, on_available: impl Fn(&[&'static str])) {
        let backoff = reprobe_backoff();

        for attempt in 1..=u8::MAX {
            let pending = self.unreachable();
            if pending.is_empty() {
                return;
            }

            let Some(delay_ms) = backoff.delay(attempt, &mut chrono_machines::rand::rng()) else {
                tracing::warn!(
                    providers = ?pending,
                    "providers still unreachable; giving up, their tools stay hidden"
                );
                return;
            };
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;

            let online: Vec<&'static str> = self
                .detect(&pending)
                .await
                .into_iter()
                .filter(|(_, status)| status.is_available())
                .map(|(name, _)| name)
                .collect();
            if !online.is_empty() {
                on_available(&online);
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&dyn Provider> {
        self.providers.get(name).map(std::convert::AsRef::as_ref)
    }

    pub fn get_arc(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    pub fn status(&self, name: &str) -> Option<ProviderStatus> {
        self.status_ref().get(name).cloned()
    }

    pub fn enabled_names(&self) -> Vec<&'static str> {
        self.status_ref().iter().filter(|(_, s)| s.is_available()).map(|(n, _)| *n).collect()
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.status_ref().get(name).is_some_and(ProviderStatus::is_available)
    }

    fn status_ref(&self) -> std::sync::RwLockReadGuard<'_, HashMap<&'static str, ProviderStatus>> {
        self.status.read().expect("provider status lock poisoned")
    }

    fn status_mut(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<&'static str, ProviderStatus>> {
        self.status.write().expect("provider status lock poisoned")
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "tea")]
pub mod forgejo;
