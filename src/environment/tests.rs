use super::*;

#[tokio::test]
async fn test_environment_outside_a_repo() {
    let dir = std::env::temp_dir().join("skipper-env-norepo");
    std::fs::create_dir_all(&dir).unwrap();

    let env = SkipperEnvironment::new(&dir).await;

    assert_eq!(env.cwd(), dir.as_path());
    assert!(!env.has_git_repo());
    // No repo means no remotes, so no forge tools regardless of installed CLIs.
    assert!(env.forges().is_empty());
    assert!(env.unknown_hosts().is_empty());
    // A missing repo reads as clean rather than erroring.
    assert!(env.git_is_clean());
    assert!(!env.git_has_staged());
}

#[tokio::test]
async fn test_forges_resolve_from_remotes() {
    let dir = std::env::temp_dir().join("skipper-env-forges");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git_in = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("git available in tests");
    };
    git_in(&["init", "-q", "."]);
    git_in(&["remote", "add", "origin", "ssh://git@192.168.3.20:12222/o/r.git"]);

    // An unmapped self-hosted host enables nothing: this is the case where an
    // installed gh must not surface GitHub tools for a Forgejo checkout.
    let env = SkipperEnvironment::new(&dir).await;
    assert!(env.has_git_repo());
    assert!(env.forges().is_empty(), "unmapped host must not enable a forge");
    assert!(env.unknown_hosts().contains("192.168.3.20"));

    // Mapping that host in config enables exactly the matching forge.
    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&std::collections::HashMap::from([(
        "192.168.3.20".to_string(),
        "forgejo".to_string(),
    )]));
    let env = SkipperEnvironment::with_hosts(&dir, hosts).await;
    assert_eq!(env.forges().iter().copied().collect::<Vec<_>>(), vec!["tea"]);
    assert!(env.unknown_hosts().is_empty());

    // A public forge remote needs no configuration.
    git_in(&["remote", "set-url", "origin", "git@github.com:o/r.git"]);
    let env = SkipperEnvironment::new(&dir).await;
    assert_eq!(env.forges().iter().copied().collect::<Vec<_>>(), vec!["github"]);

    let _ = std::fs::remove_dir_all(&dir);
}
