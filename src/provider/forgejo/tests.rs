use super::*;

const CONFIG: &str = r#"
logins:
    - name: stale
      url: http://192.168.3.20:13000
      token: ""
      default: false
    - name: forgero
      url: http://192.168.3.20:13000/
      token: abc123
      default: true
    - name: codeberg
      url: https://codeberg.org
      token: def456
      default: false
preferences:
    editor: false
"#;

fn config_file(yaml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("config.yml");
    std::fs::write(&file, yaml).expect("write tea config");
    (dir, file)
}

fn logins(yaml: &str) -> Vec<Credentials> {
    let (_dir, file) = config_file(yaml);
    load_credentials_from(&file)
}

#[test]
fn test_reads_tea_logins() {
    let creds = logins(CONFIG);

    assert_eq!(creds.len(), 2);
    assert_eq!(creds[0].name, "forgero");
    assert_eq!(creds[0].url, "http://192.168.3.20:13000");
    assert_eq!(creds[0].token, "abc123");
}

#[test]
fn test_credentials_matched_by_host() {
    let creds = logins(CONFIG);

    let c = match_host(&creds, "192.168.3.20").expect("self-hosted login found");
    assert_eq!(c.name, "forgero");

    let c = match_host(&creds, "CodeBerg.org").expect("public login found, any case");
    assert_eq!(c.token, "def456");

    assert!(match_host(&creds, "github.com").is_none());
}

#[test]
fn test_tokenless_login_never_shadows_a_live_one() {
    let creds = logins(CONFIG);
    let c = match_host(&creds, "192.168.3.20").expect("live login found");
    assert_eq!(c.token, "abc123");
}

#[test]
fn test_missing_or_broken_config_is_not_fatal() {
    assert!(load_credentials_from(std::path::Path::new("/nonexistent/tea.yml")).is_empty());

    assert!(logins("logins: [ this is not: valid yaml").is_empty());
    assert!(logins("preferences:\n    editor: false\n").is_empty());
}
