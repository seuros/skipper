use super::*;

#[test]
fn test_rollup_keeps_latest_attempt_and_buckets_like_gh() {
    let contexts: Vec<Context> = serde_json::from_str(
        r#"[
        {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE",
         "detailsUrl":"https://x/1","title":"","startedAt":"2026-10-01T10:00:00Z",
         "checkSuite":{"workflowRun":{"workflow":{"name":"CI"}}}},
        {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS",
         "detailsUrl":"https://x/2","title":null,"startedAt":"2026-10-01T11:00:00Z",
         "checkSuite":{"workflowRun":{"workflow":{"name":"CI"}}}},
        {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"NEUTRAL",
         "detailsUrl":null,"title":null,"startedAt":null,"checkSuite":null},
        {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":null,
         "detailsUrl":null,"title":null,"startedAt":null,"checkSuite":null},
        {"__typename":"StatusContext","context":"codecov","state":"ERROR",
         "targetUrl":"","description":"coverage dropped","createdAt":"2026-10-01T11:00:00Z"}
    ]"#,
    )
    .expect("rollup nodes");

    let checks = latest(contexts);
    let summary: Vec<(&str, &str, &str)> =
        checks.iter().map(|c| (c.workflow.as_str(), c.name.as_str(), c.bucket.as_str())).collect();
    assert_eq!(
        summary,
        [
            ("CI", "test", "pass"),
            ("", "docs", "skipping"),
            ("", "lint", "pending"),
            ("", "codecov", "fail")
        ]
    );
    assert_eq!(checks[0].link.as_deref(), Some("https://x/2"));
    assert_eq!(
        (checks[3].link.as_deref(), checks[3].description.as_deref()),
        (None, Some("coverage dropped"))
    );
}
