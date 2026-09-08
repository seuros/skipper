use super::*;

#[test]
fn test_tea_provider_config() {
    let provider = TeaProvider::new();
    assert_eq!(provider.name(), "tea");
    assert_eq!(provider.cli(), "tea");
    assert!(provider.min_version() >= Version::new(0, 9, 0));
}

#[test]
fn test_repo_list_shape() {
    let json = r#"[{"id":"158","owner":"seuros","name":"skipper","type":"source",
        "description":"desc","url":"http://host/seuros/skipper",
        "ssh":"ssh://git@host/seuros/skipper.git","permission":"admin"}]"#;
    let repos: Vec<Repository> = serde_json::from_str(json).unwrap();
    assert_eq!(repos[0].owner, "seuros");
    assert_eq!(repos[0].id.as_deref(), Some("158"));
}

#[test]
fn test_issue_list_shape() {
    let json = r#"[{"index":"1","title":"t","state":"open","author":"seuros",
        "body":"b","created":"2026-07-23T23:58:31Z","updated":"2026-07-23T23:58:31Z",
        "labels":"bug, feature","url":"http://host/o/r/issues/1"}]"#;
    let issues: Vec<Issue> = serde_json::from_str(json).unwrap();
    assert_eq!(issues[0].index_u64(), Some(1));
    assert_eq!(issues[0].label_names(), vec!["bug", "feature"]);
}

#[test]
fn test_issue_detail_shape() {
    let json = r#"{"id":1,"index":1,"title":"t","state":"open",
        "created":"2026-07-23T23:58:31Z","labels":[],"user":"seuros","body":"b",
        "assignees":[],"url":"http://host/o/r/issues/1","closedAt":null,"comments":[]}"#;
    let issue: IssueDetail = serde_json::from_str(json).unwrap();
    assert_eq!(issue.index, 1);
    assert_eq!(issue.user, "seuros");
    assert!(issue.closed_at.is_none());
}

#[test]
fn test_pull_detail_shape() {
    let json = r#"{"id":1,"index":2,"title":"t","state":"open",
        "created":"2026-07-24T00:02:06Z","updated":"2026-07-24T00:02:06Z","labels":[],
        "user":"seuros","body":"b","assignees":[],"url":"http://host/o/r/pulls/2",
        "base":"main","head":"fix","headSha":"c6bea","diffUrl":"http://host/o/r/pulls/2.diff",
        "mergeable":true,"hasMerged":false,"mergedAt":null,"closedAt":null,
        "reviews":[],"comments":[]}"#;
    let pr: PullRequestDetail = serde_json::from_str(json).unwrap();
    assert_eq!(pr.index, 2);
    assert_eq!(pr.mergeable, Some(true));
    assert!(!pr.has_merged);
}

#[test]
fn test_branch_shape() {
    let json = r#"[{"name":"main","protected":"false",
        "user-can-merge":"true","user-can-push":"true"}]"#;
    let branches: Vec<Branch> = serde_json::from_str(json).unwrap();
    assert_eq!(branches[0].name, "main");
    assert!(!branches[0].is_protected());
}

#[test]
fn test_release_shape() {
    let json = r#"[{"tag-_name":"v0.0.1","title":"r","published _at":"2026-07-23T23:58:21Z",
        "status":"released","tar/_zip url":"http://host/a.tar.gz\nhttp://host/a.zip"}]"#;
    let releases: Vec<Release> = serde_json::from_str(json).unwrap();
    assert_eq!(releases[0].tag_name, "v0.0.1");
    assert_eq!(releases[0].status.as_deref(), Some("released"));
}

#[test]
fn test_login_shape() {
    let json = r#"[{"name":"gitea-local","url":"http://host","ssh_host":"host",
        "user":"seuros","default":"true"}]"#;
    let logins: Vec<Login> = serde_json::from_str(json).unwrap();
    assert!(logins[0].is_default());
    assert_eq!(logins[0].user, "seuros");
}
