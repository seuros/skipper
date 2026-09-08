use skipper::provider::{Provider, ProviderStatus, Registry};

#[cfg(feature = "github")]
use skipper::provider::github::GitHubProvider;

#[cfg(feature = "tea")]
use skipper::provider::tea::TeaProvider;

#[cfg(feature = "gitlab")]
use skipper::provider::gitlab::GitLabProvider;

#[test]
fn test_registry_creation() {
    let registry = Registry::with_defaults();

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
            println!("GitHub CLI available: v{}", version);
            assert!(version >= provider.min_version());
        }
        ProviderStatus::NotInstalled => {
            println!("GitHub CLI (gh) not installed - skipping");
        }
        ProviderStatus::VersionTooLow { found, required } => {
            println!("GitHub CLI version {} < required {} - skipping", found, required);
        }
        ProviderStatus::AuthRequired => {
            println!("GitHub CLI not authenticated - run 'gh auth login'");
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
            println!("Tea CLI available: v{}", version);
            assert!(version >= provider.min_version());
        }
        ProviderStatus::NotInstalled => {
            println!("Tea CLI not installed - skipping");
        }
        ProviderStatus::VersionTooLow { found, required } => {
            println!("Tea CLI version {} < required {} - skipping", found, required);
        }
        ProviderStatus::AuthRequired => {
            println!("Tea CLI not authenticated - run 'tea login'");
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
            println!("GitLab CLI available: v{}", version);
            assert!(version >= provider.min_version());
        }
        ProviderStatus::NotInstalled => {
            println!("GitLab CLI (glab) not installed - skipping");
        }
        ProviderStatus::VersionTooLow { found, required } => {
            println!("GitLab CLI version {} < required {} - skipping", found, required);
        }
        ProviderStatus::AuthRequired => {
            println!("GitLab CLI not authenticated - run 'glab auth login'");
        }
    }
}

#[tokio::test]
async fn test_registry_detect_all() {
    let mut registry = Registry::with_defaults();
    let results = registry.detect_all().await;

    println!("\n=== Provider Detection Results ===");
    for (name, status) in &results {
        match status {
            ProviderStatus::Available { version } => {
                println!("✓ {}: v{}", name, version);
            }
            ProviderStatus::NotInstalled => {
                println!("✗ {}: not installed", name);
            }
            ProviderStatus::VersionTooLow { found, required } => {
                println!("✗ {}: v{} < v{} (outdated)", name, found, required);
            }
            ProviderStatus::AuthRequired => {
                println!("! {}: not authenticated", name);
            }
        }
    }
    println!("===================================\n");

    let enabled: Vec<_> = registry.enabled_names();
    println!("Enabled providers: {:?}", enabled);

    for name in &enabled {
        assert!(registry.is_enabled(name));
        let status = registry.status(name).unwrap();
        assert!(status.is_available());
    }
}

#[tokio::test]
async fn test_enabled_providers_execute() {
    let mut registry = Registry::with_defaults();
    registry.detect_all().await;

    for provider in registry.enabled() {
        println!("Testing {} CLI execution...", provider.name());

        let result = provider.execute(&["--version"]).await;
        assert!(result.is_ok(), "Failed to execute {} --version", provider.cli());

        let output = result.unwrap();
        assert!(output.success());
        println!("  {} --version: OK", provider.cli());
    }
}
