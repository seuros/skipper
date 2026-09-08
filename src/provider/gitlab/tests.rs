use super::*;

#[test]
fn test_gitlab_provider_config() {
    let provider = GitLabProvider::new();
    assert_eq!(provider.name(), "gitlab");
    assert_eq!(provider.cli(), "glab");
    assert!(provider.min_version() >= Version::new(1, 0, 0));
}

#[test]
fn test_pipeline_normalization() {
    let json = r#"[
        {"id": 1001, "status": "success", "ref": "master",
         "web_url": "https://gitlab.com/g/p/-/pipelines/1001"},
        {"id": 1002, "status": "failed", "ref": "master", "web_url": null},
        {"id": 1003, "status": "running", "ref": "feature", "web_url": null},
        {"id": 1004, "status": "waiting_for_resource", "ref": null, "web_url": null},
        {"id": 1005, "status": "canceled", "ref": null, "web_url": null}
    ]"#;
    let pipelines: Vec<Pipeline> = serde_json::from_str(json).unwrap();
    let runs: Vec<BuildRun> = pipelines.into_iter().map(Into::into).collect();

    assert_eq!(runs[0].status, "success");
    assert!(runs[0].is_terminal());
    assert_eq!(runs[1].status, "failure");
    assert_eq!(runs[2].status, "running");
    assert!(!runs[2].is_terminal());
    assert_eq!(runs[3].status, "queued");
    assert_eq!(runs[4].status, "cancelled");
    assert!(runs[4].is_terminal());
}
