use super::super::commit_checks::{CheckRuns, CombinedStatus};
use super::*;
use std::assert_matches;

#[test]
fn missing_rollup_reads_head_runs_and_actual_statuses() {
    for rollup in [
        serde_json::Value::Null,
        serde_json::json!({"contexts":{"nodes":[],"pageInfo":{"hasNextPage":false}}}),
    ] {
        let pull: Pull = serde_json::from_value(serde_json::json!({
            "headRefOid":"release-head",
            "commits":{"nodes":[{"commit":{"oid":"release-head","statusCheckRollup":rollup}}]}
        }))
        .unwrap();
        assert_matches!(
            rollup_page(pull.commits, &pull.head_ref_oid, None, &mut vec![]),
            Ok(PageAction::ReadCommit)
        );
    }
    let runs: CheckRuns = serde_json::from_str(r#"{"check_runs":[
        {"name":"driver / Dictator and tests","status":"completed","conclusion":"success","html_url":"job"}
    ]}"#).unwrap();
    let mut checks: Vec<PrCheck> = runs.check_runs.into_iter().map(Into::into).collect();
    for (statuses, expected) in [
        (r#"{"state":"pending","statuses":[]}"#, "success"),
        (r#"{"state":"failure","statuses":[{"context":"external","state":"error"}]}"#, "failure"),
    ] {
        let page: CombinedStatus = serde_json::from_str(statuses).unwrap();
        checks.extend(page.statuses.into_iter().map(PrCheck::from));
        assert_eq!(super::super::CheckCounts::tally(&checks).conclusion(), expected);
    }
    assert_eq!(checks[0].link.as_deref(), Some("job"));
}

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
        checks.iter().map(|c| (c.workflow.as_str(), c.name.as_str(), c.bucket)).collect();
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
