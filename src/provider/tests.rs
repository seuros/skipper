use super::*;

#[test]
fn test_provider_status() {
    let available = ProviderStatus::Available { version: Version::new(1, 0, 0) };
    assert!(available.is_available());
    assert_eq!(available.version(), Some(&Version::new(1, 0, 0)));

    let not_installed = ProviderStatus::NotInstalled;
    assert!(!not_installed.is_available());
    assert_eq!(not_installed.version(), None);
}

#[test]
fn test_registry_new() {
    let registry = Registry::new();
    assert!(registry.enabled_names().is_empty());
}

#[cfg(any(feature = "github", feature = "gitlab"))]
#[test]
fn test_auth_network_failure_is_not_logged_out() {
    assert!(network_failure("X Timeout trying to log in to github.com account seuros (keyring)"));
    assert!(network_failure("x gitlab.com: API call failed: read tcp: connection reset by peer"));
    assert!(!network_failure(
        "You are not logged into any GitHub hosts. To log in, run: gh auth login"
    ));
    assert!(!network_failure("x gitlab.com: API call failed: 401 Unauthorized"));
}

/// Unreachable for two probes, then available.
#[derive(Default)]
struct Flaky(std::sync::atomic::AtomicU8);

impl Provider for Flaky {
    fn name(&self) -> &'static str {
        "flaky"
    }
    fn cli(&self) -> &'static str {
        "flaky"
    }
    fn min_version(&self) -> Version {
        Version::new(1, 0, 0)
    }
    fn check_auth(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async { Ok(true) })
    }
    fn detect(&self) -> BoxFuture<'_, ProviderStatus> {
        let up = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2;
        let version = Version::new(1, 0, 0);
        Box::pin(async move {
            if up { ProviderStatus::Available { version } } else { ProviderStatus::Unreachable }
        })
    }
}

#[tokio::test(start_paused = true)]
async fn test_unreachable_provider_recovers_through_retry() {
    let mut registry = Registry::new();
    registry.register(Box::new(Flaky::default()));
    registry.detect_all().await;
    assert_eq!(registry.unreachable(), ["flaky"]);

    let online = std::sync::Mutex::new(Vec::new());
    registry.retry_unreachable(|names| online.lock().unwrap().extend_from_slice(names)).await;
    assert_eq!(*online.lock().unwrap(), ["flaky"]);
    assert!(registry.is_enabled("flaky"));
}

#[test]
fn test_conclusion_of_runs() {
    let run = |status: &str| BuildRun {
        id: "1".into(),
        status: status.into(),
        branch: None,
        workflow: None,
        title: None,
        url: None,
    };
    assert_eq!(conclusion_of(&[]), "no_runs");
    assert_eq!(conclusion_of(&[run("success"), run("skipped"), run("completed")]), "success");
    assert_eq!(conclusion_of(&[run("success"), run("running")]), "pending");
    assert_eq!(conclusion_of(&[run("cancelled"), run("queued")]), "cancelled");
    assert_eq!(conclusion_of(&[run("cancelled"), run("failure"), run("running")]), "failure");
}
