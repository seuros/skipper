use super::*;

#[test]
fn test_parse_config() {
    let toml = r#"
default_provider = "gitlab"

[remotes.origin]
provider = "gitea"
url = "https://git.example.com"

[providers.gitlab]
url = "https://gitlab.company.com"
default_org = "myteam"
"#;

    let config: Config = toml::from_str(toml).unwrap();

    assert_eq!(config.default_provider, Some("gitlab".to_string()));

    let origin = config.remote("origin").unwrap();
    assert_eq!(origin.provider, "gitea");
    assert_eq!(origin.url, Some("https://git.example.com".to_string()));

    let gitlab = config.providers.gitlab.unwrap();
    assert_eq!(gitlab.url, Some("https://gitlab.company.com".to_string()));
    assert_eq!(gitlab.default_org, Some("myteam".to_string()));
}

#[test]
fn test_default_config() {
    let config = Config::default();
    assert!(config.default_provider.is_none());
    assert!(config.remotes.is_empty());
}

#[test]
fn test_writes_come_from_the_global_config_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("skipper.toml");
    let global = dir.path().join("config.toml");
    std::fs::write(&local, "[writes]\nenabled = true\nconfirm = false\n").expect("local");

    let repo_only = Config::load_with(&local, Some(&global));
    assert_eq!(repo_only.writes, WritesConfig { enabled: false, confirm: true });

    std::fs::write(&global, "[writes]\nenabled = true\n").expect("global");
    let user = Config::load_with(&local, Some(&global));
    assert_eq!(user.writes, WritesConfig { enabled: true, confirm: true });
}
