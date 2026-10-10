use crate::provider::{BoxFuture, Provider};
use crate::version::minimum;
use semver::Version;

pub struct TeaProvider {
    min_version: Version,
}

impl TeaProvider {
    pub const fn new() -> Self {
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

    fn check_auth(&self) -> BoxFuture<'_, crate::error::Result<bool>> {
        Box::pin(async move {
            let Some(creds) = super::forgejo::any_credentials() else {
                tracing::debug!("no tea login with a token; forge tools stay hidden");
                return Ok(false);
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
                    Ok(true)
                }
                // Network failure or timeout, or the server erroring: unknown.
                Err(
                    e @ (crate::error::CliError::Io { .. }
                    | crate::error::CliError::ExecutionFailed { code: 500.., .. }),
                ) => Err(e),
                Err(e) => {
                    tracing::warn!(
                        login = client.login_name(),
                        error = %e,
                        "forgejo token rejected"
                    );
                    Ok(false)
                }
            }
        })
    }
}
