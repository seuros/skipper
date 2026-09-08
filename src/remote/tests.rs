use super::*;

#[test]
fn test_host_of_url_shapes() {
    assert_eq!(host_of("https://github.com/seuros/repo.git").as_deref(), Some("github.com"));
    assert_eq!(host_of("git@github.com:seuros/repo.git").as_deref(), Some("github.com"));
    assert_eq!(
        host_of("ssh://git@192.168.3.20:12222/seuros/skipper.git").as_deref(),
        Some("192.168.3.20")
    );
    assert_eq!(
        host_of("https://user:token@gitlab.example.com/group/project.git").as_deref(),
        Some("gitlab.example.com")
    );
    assert_eq!(host_of("git://Git.Example.COM/repo").as_deref(), Some("git.example.com"));
    assert_eq!(host_of("ssh://git@[2001:db8::1]:2222/repo.git").as_deref(), Some("2001:db8::1"));
}

#[test]
fn test_host_of_rejects_local_paths() {
    assert_eq!(host_of("/srv/git/repo.git"), None);
    assert_eq!(host_of("./sub/repo"), None);
    assert_eq!(host_of("../sibling.git"), None);
    assert_eq!(host_of("~/repos/thing.git"), None);
    assert_eq!(host_of("file:///srv/git/repo.git"), None);
    assert_eq!(host_of(""), None);
    assert_eq!(host_of("sub/dir:name"), None);
}

#[test]
fn test_normalize_provider_aliases() {
    assert_eq!(normalize_provider("gh"), Some("github"));
    assert_eq!(normalize_provider("GitHub"), Some("github"));
    assert_eq!(normalize_provider("glab"), Some("gitlab"));
    assert_eq!(normalize_provider("forgejo"), Some("tea"));
    assert_eq!(normalize_provider("gitea"), Some("tea"));
    assert_eq!(normalize_provider(" tea "), Some("tea"));
    assert_eq!(normalize_provider("bitbucket"), None);
}

#[test]
fn test_default_hosts() {
    let hosts = ForgeHosts::with_defaults();
    assert_eq!(hosts.provider_for_url("git@github.com:o/r.git"), Some("github"));
    assert_eq!(hosts.provider_for_url("https://gitlab.com/g/p"), Some("gitlab"));
    assert_eq!(hosts.provider_for_url("https://codeberg.org/o/r"), Some("tea"));
    assert_eq!(hosts.provider_for_url("ssh://git@192.168.3.20:12222/o/r.git"), None);
}

#[test]
fn test_configured_self_hosted_instances() {
    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&HashMap::from([
        ("github.enterprise.com".to_string(), "github".to_string()),
        ("192.168.3.20".to_string(), "forgejo".to_string()),
        ("https://gitlab.internal:8443".to_string(), "glab".to_string()),
        ("bad.example.com".to_string(), "bitbucket".to_string()),
    ]));

    assert_eq!(hosts.provider_for_url("git@github.enterprise.com:o/r.git"), Some("github"));
    assert_eq!(hosts.provider_for_url("ssh://git@192.168.3.20:12222/o/r.git"), Some("tea"));
    assert_eq!(hosts.provider_for_url("https://gitlab.internal/g/p.git"), Some("gitlab"));
    assert_eq!(hosts.provider_for_url("https://bad.example.com/o/r"), None);
    assert_eq!(hosts.provider_for_url("https://github.com/o/r"), Some("github"));
}

#[test]
fn test_config_overrides_default_host() {
    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&HashMap::from([("codeberg.org".to_string(), "github".to_string())]));
    assert_eq!(hosts.provider_for_url("https://codeberg.org/o/r"), Some("github"));
}

#[test]
fn test_providers_for_urls() {
    let mut hosts = ForgeHosts::with_defaults();
    hosts.extend(&HashMap::from([("192.168.3.20".to_string(), "forgejo".to_string())]));

    let providers = hosts.providers_for_urls(["ssh://git@192.168.3.20:12222/seuros/skipper.git"]);
    assert_eq!(providers.into_iter().collect::<Vec<_>>(), vec!["tea"]);

    let providers = hosts.providers_for_urls([
        "git@github.com:seuros/skipper.git",
        "ssh://git@192.168.3.20:12222/seuros/skipper.git",
    ]);
    assert_eq!(providers.into_iter().collect::<Vec<_>>(), vec!["github", "tea"]);

    assert!(hosts.providers_for_urls(["/srv/git/repo.git"]).is_empty());
}

#[test]
fn test_unknown_hosts_reported() {
    let hosts = ForgeHosts::with_defaults();
    let unknown = hosts.unknown_hosts([
        "https://github.com/o/r",
        "ssh://git@192.168.3.20:12222/o/r.git",
        "/srv/git/local.git",
    ]);
    assert_eq!(unknown.into_iter().collect::<Vec<_>>(), vec!["192.168.3.20".to_string()]);
}

#[test]
fn test_repo_path_of() {
    assert_eq!(
        repo_path_of("ssh://git@192.168.3.20:12222/seuros/skipper.git"),
        Some(("seuros".into(), "skipper".into()))
    );
    assert_eq!(
        repo_path_of("git@github.com:seuros/skipper-ci-trash.git"),
        Some(("seuros".into(), "skipper-ci-trash".into()))
    );
    assert_eq!(
        repo_path_of("https://github.com/seuros/repo"),
        Some(("seuros".into(), "repo".into()))
    );
    assert_eq!(
        repo_path_of("https://gitlab.com/group/subgroup/project.git"),
        Some(("subgroup".into(), "project".into()))
    );
    assert_eq!(
        repo_path_of("https://user:token@git.example.com/o/r.git"),
        Some(("o".into(), "r".into()))
    );
}

#[test]
fn test_repo_path_of_rejects_incomplete_urls() {
    assert_eq!(repo_path_of("https://github.com/lonely.git"), None);
    assert_eq!(repo_path_of("https://github.com/"), None);
    assert_eq!(repo_path_of(""), None);
    assert_eq!(repo_path_of("/srv/git/repo.git"), None);
}
