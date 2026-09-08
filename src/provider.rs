#[cfg(feature = "github")]
pub mod github;

#[cfg(feature = "tea")]
pub mod tea;

#[cfg(feature = "gitlab")]
pub mod gitlab;

use crate::error::{CliError, Result};
use crate::executor;
use crate::version;
use semver::Version;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub enum ProviderStatus {
    Available { version: Version },
    NotInstalled,
    VersionTooLow { found: Version, required: Version },
    AuthRequired,
}

impl ProviderStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub fn version(&self) -> Option<&Version> {
        match self {
            Self::Available { version } => Some(version),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct BuildRun {
    pub id: String,
    pub status: String,
    pub branch: Option<String>,
    pub workflow: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
}

impl BuildRun {
    pub fn is_terminal(&self) -> bool {
        is_terminal_status(&self.status)
    }
}

pub fn is_terminal_status(status: &str) -> bool {
    matches!(status, "success" | "failure" | "cancelled" | "skipped" | "completed")
}

#[cfg(any(feature = "github", feature = "gitlab"))]
async fn cli_authenticated(cli: &str) -> bool {
    executor::execute(cli, &["auth", "status"], Duration::from_secs(5))
        .await
        .map(|o| o.success())
        .unwrap_or(false)
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;

    fn cli(&self) -> &'static str;

    fn min_version(&self) -> Version;

    fn detect(&self) -> BoxFuture<'_, ProviderStatus> {
        Box::pin(async move {
            let version_output = match executor::get_version_output(self.cli()).await {
                Ok(output) => output,
                Err(CliError::NotInstalled { .. }) => return ProviderStatus::NotInstalled,
                Err(_) => return ProviderStatus::NotInstalled,
            };

            let version = match version::parse_version(&version_output, self.cli()) {
                Ok(v) => v,
                Err(_) => return ProviderStatus::NotInstalled,
            };

            if version < self.min_version() {
                return ProviderStatus::VersionTooLow {
                    found: version,
                    required: self.min_version(),
                };
            }

            if !self.check_auth().await {
                return ProviderStatus::AuthRequired;
            }

            ProviderStatus::Available { version }
        })
    }

    fn check_auth(&self) -> BoxFuture<'_, bool>;

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

    fn ci_runs(&self, _limit: usize) -> BoxFuture<'_, Result<Vec<BuildRun>>> {
        Box::pin(async move { Err(CliError::unsupported(self.cli(), "CI run listing")) })
    }

    fn ci_run<'a>(&'a self, _id: Option<&'a str>) -> BoxFuture<'a, Result<BuildRun>> {
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

pub struct Registry {
    providers: HashMap<&'static str, Arc<dyn Provider>>,
    status: HashMap<&'static str, ProviderStatus>,
}

impl Registry {
    pub fn new() -> Self {
        Self { providers: HashMap::new(), status: HashMap::new() }
    }

    pub fn with_defaults() -> Self {
        #[allow(unused_mut)]
        let mut registry = Self::new();

        #[cfg(feature = "github")]
        registry.register(Box::new(github::GitHubProvider::new()));

        #[cfg(feature = "tea")]
        registry.register(Box::new(tea::TeaProvider::new()));

        #[cfg(feature = "gitlab")]
        registry.register(Box::new(gitlab::GitLabProvider::new()));

        registry
    }

    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.insert(provider.name(), Arc::from(provider));
    }

    pub async fn detect_all(&mut self) -> Vec<(&'static str, ProviderStatus)> {
        let mut results = Vec::new();

        for (name, provider) in &self.providers {
            let status = provider.detect().await;
            tracing::info!(
                provider = name,
                status = ?status,
                "provider detection complete"
            );
            self.status.insert(name, status.clone());
            results.push((*name, status));
        }

        results
    }

    pub fn get(&self, name: &str) -> Option<&dyn Provider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    pub fn get_arc(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    pub fn status(&self, name: &str) -> Option<&ProviderStatus> {
        self.status.get(name)
    }

    pub fn enabled(&self) -> impl Iterator<Item = &dyn Provider> {
        self.providers
            .iter()
            .filter(|(name, _)| self.status.get(*name).map(|s| s.is_available()).unwrap_or(false))
            .map(|(_, p)| p.as_ref())
    }

    pub fn enabled_names(&self) -> Vec<&'static str> {
        self.enabled().map(|p| p.name()).collect()
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.status.get(name).map(|s| s.is_available()).unwrap_or(false)
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
