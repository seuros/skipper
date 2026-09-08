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
