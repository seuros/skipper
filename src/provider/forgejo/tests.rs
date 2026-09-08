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

fn config_file(name: &str, yaml: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("skipper-forgejo-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.yml");
    std::fs::write(&file, yaml).unwrap();
    file
}

#[test]
fn test_reads_tea_logins() {
    let creds = load_credentials_from(&config_file("read", CONFIG));

    assert_eq!(creds.len(), 2);
    assert_eq!(creds[0].name, "forgero");
    assert_eq!(creds[0].url, "http://192.168.3.20:13000");
    assert_eq!(creds[0].token, "abc123");
}

#[test]
fn test_credentials_matched_by_host() {
    let creds = || load_credentials_from(&config_file("host", CONFIG));

    let c = match_host(creds(), "192.168.3.20").expect("self-hosted login found");
    assert_eq!(c.name, "forgero");

    let c = match_host(creds(), "codeberg.org").expect("public login found");
    assert_eq!(c.token, "def456");

    assert!(match_host(creds(), "github.com").is_none());
}

#[test]
fn test_tokenless_login_never_shadows_a_live_one() {
    let creds = load_credentials_from(&config_file("shadow", CONFIG));
    let c = match_host(creds, "192.168.3.20").unwrap();
    assert_eq!(c.token, "abc123");
}

#[test]
fn test_missing_or_broken_config_is_not_fatal() {
    assert!(load_credentials_from(std::path::Path::new("/nonexistent/tea.yml")).is_empty());

    let broken = config_file("broken", "logins: [ this is not: valid yaml");
    assert!(load_credentials_from(&broken).is_empty());

    let empty = config_file("empty", "preferences:\n    editor: false\n");
    assert!(load_credentials_from(&empty).is_empty());
}
