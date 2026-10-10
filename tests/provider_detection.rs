// Every test probes a forge CLI; a forge-less build has nothing to detect.
#![cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]

use skipper::provider::{Provider, ProviderStatus, Registry};

#[cfg(feature = "github")]
use skipper::provider::github::GitHubProvider;

#[cfg(feature = "tea")]
use skipper::provider::tea::TeaProvider;

#[cfg(feature = "gitlab")]
use skipper::provider::gitlab::GitLabProvider;

#[test]
fn test_registry_creation() {
    let registry = Registry::with_defaults(|_| false);

    #[cfg(feature = "github")]
    assert!(registry.get("github").is_some());

    #[cfg(feature = "tea")]
    assert!(registry.get("tea").is_some());

    #[cfg(feature = "gitlab")]
    assert!(registry.get("gitlab").is_some());
}

#[cfg(feature = "github")]
#[tokio::test]
async fn test_github_detection() {
    let provider = GitHubProvider::new();

    assert_eq!(provider.name(), "github");
    assert_eq!(provider.cli(), "gh");

    let status = provider.detect().await;

    match status {
        ProviderStatus::Available { version } => {
            println!("GitHub CLI available: v{version}");
            assert!(version >= provider.min_version());
        }
        ProviderStatus::NotInstalled => {
            println!("GitHub CLI (gh) not installed - skipping");
        }
        ProviderStatus::VersionTooLow { found, required } => {
            println!("GitHub CLI version {found} < required {required} - skipping");
        }
        ProviderStatus::AuthRequired => {
            println!("GitHub CLI not authenticated - run 'gh auth login'");
        }
        ProviderStatus::Unreachable => {
            println!("probe timed out or network failed - skipping");
        }
    }
}

#[cfg(feature = "tea")]
#[tokio::test]
async fn test_tea_detection() {
    let provider = TeaProvider::new();

    assert_eq!(provider.name(), "tea");
    assert_eq!(provider.cli(), "tea");

    let status = provider.detect().await;

    match status {
        ProviderStatus::Available { version } => {
            println!("Tea CLI available: v{version}");
            assert!(version >= provider.min_version());
        }
        ProviderStatus::NotInstalled => {
            println!("Tea CLI not installed - skipping");
        }
        ProviderStatus::VersionTooLow { found, required } => {
            println!("Tea CLI version {found} < required {required} - skipping");
        }
        ProviderStatus::AuthRequired => {
            println!("Tea CLI not authenticated - run 'tea login'");
        }
        ProviderStatus::Unreachable => {
            println!("probe timed out or network failed - skipping");
        }
    }
}

#[cfg(feature = "gitlab")]
#[tokio::test]
async fn test_gitlab_detection() {
    let provider = GitLabProvider::new();

    assert_eq!(provider.name(), "gitlab");
    assert_eq!(provider.cli(), "glab");

    let status = provider.detect().await;

    match status {
        ProviderStatus::Available { version } => {
            println!("GitLab CLI available: v{version}");
            assert!(version >= provider.min_version());
        }
        ProviderStatus::NotInstalled => {
            println!("GitLab CLI (glab) not installed - skipping");
        }
        ProviderStatus::VersionTooLow { found, required } => {
            println!("GitLab CLI version {found} < required {required} - skipping");
        }
        ProviderStatus::AuthRequired => {
            println!("GitLab CLI not authenticated - run 'glab auth login'");
        }
        ProviderStatus::Unreachable => {
            println!("probe timed out or network failed - skipping");
        }
    }
}

#[tokio::test]
async fn test_registry_detect_all() {
    let registry = Registry::with_defaults(|_| false);
    let results = registry.detect_all().await;

    println!("\n=== Provider Detection Results ===");
    for (name, status) in &results {
        match status {
            ProviderStatus::Available { version } => {
                println!("✓ {name}: v{version}");
            }
            ProviderStatus::NotInstalled => {
                println!("✗ {name}: not installed");
            }
            ProviderStatus::VersionTooLow { found, required } => {
                println!("✗ {name}: v{found} < v{required} (outdated)");
            }
            ProviderStatus::AuthRequired => {
                println!("! {name}: not authenticated");
            }
            ProviderStatus::Unreachable => {
                println!("probe timed out or network failed - skipping");
            }
        }
    }
    println!("===================================\n");

    let enabled: Vec<_> = registry.enabled_names();
    println!("Enabled providers: {enabled:?}");

    for name in &enabled {
        assert!(registry.is_enabled(name));
        let status = registry.status(name).unwrap();
        assert!(status.is_available());
    }
}

#[tokio::test]
async fn test_enabled_providers_execute() {
    let registry = Registry::with_defaults(|_| false);
    registry.detect_all().await;

    for provider in registry.enabled_names().into_iter().filter_map(|name| registry.get(name)) {
        println!("Testing {} CLI execution...", provider.name());

        let result = provider.execute(&["--version"]).await;
        assert!(result.is_ok(), "Failed to execute {} --version", provider.cli());

        let output = result.unwrap();
        assert!(output.success());
        println!("  {} --version: OK", provider.cli());
    }
}
