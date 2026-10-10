use super::*;
use crate::git::test_support::git;

#[test]
fn test_environment_outside_a_repo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let env = SkipperEnvironment::with_hosts(temp.path(), ForgeHosts::with_defaults());

    assert_eq!(env.cwd(), temp.path());
    assert!(!env.has_git_repo());
    // No repo means no remotes, so no forge tools regardless of installed CLIs.
    assert!(env.forges().is_empty());
    assert!(env.unknown_hosts().is_empty());
    // A missing repo reads as clean rather than erroring.
    assert!(env.git_is_clean());
    assert!(!env.git_has_staged());
}

#[test]
fn test_forges_resolve_from_remotes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path();
    git(dir, &["init", "-q", "."]);
    git(dir, &["remote", "add", "origin", "ssh://git@192.168.3.20:12222/o/r.git"]);

    // An unmapped self-hosted host enables nothing: this is the case where an
    // installed gh must not surface GitHub tools for a Forgejo checkout.
    let env = SkipperEnvironment::with_hosts(dir, ForgeHosts::with_defaults());
    assert!(env.has_git_repo());
    assert!(env.forges().is_empty(), "unmapped host must not enable a forge");
    assert!(env.unknown_hosts().contains("192.168.3.20"));

    // Mapping that host in config enables exactly the matching forge.
    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&std::collections::HashMap::from([(
        "192.168.3.20".to_string(),
        "forgejo".to_string(),
    )]));
    let env = SkipperEnvironment::with_hosts(dir, hosts);
    assert_eq!(env.forges().into_iter().collect::<Vec<_>>(), ["tea"]);
    assert!(env.unknown_hosts().is_empty());

    // A public forge remote needs no configuration, and a refresh sees a
    // switched URL with its current remote.
    git(dir, &["remote", "set-url", "origin", "git@github.com:o/r.git"]);
    assert!(env.refresh());
    assert!(!env.has_forge("tea"));
    let env = SkipperEnvironment::with_hosts(dir, ForgeHosts::with_defaults());
    assert_eq!(env.forges().into_iter().collect::<Vec<_>>(), ["github"]);
    assert_eq!(env.current_remote_label().as_deref(), Some("origin (github)"));
}
