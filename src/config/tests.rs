use super::*;

#[test]
fn test_disabled_providers_map_to_forges() {
    // Forgejo and Gitea are one forge (tea); keys skipper never read still parse.
    let config: Config = toml::from_str(
        "[providers.forgejo]\ndisabled = true\nurl = \"https://old.example\"\n\
         [providers.github]\ndisabled = false\n",
    )
    .expect("config");

    let disabled: Vec<&str> =
        ["github", "gitlab", "tea"].into_iter().filter(|f| config.is_disabled(f)).collect();
    assert_eq!(disabled, ["tea"]);
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
