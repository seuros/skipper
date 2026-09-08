use super::*;

#[test]
fn test_github_provider_config() {
    let provider = GitHubProvider::new();
    assert_eq!(provider.name(), "github");
    assert_eq!(provider.cli(), "gh");
    assert!(provider.min_version() >= Version::new(2, 0, 0));
}

#[test]
fn test_workflow_run_normalization() {
    let json = r#"[
        {"databaseId": 42, "status": "completed", "conclusion": "success",
         "headBranch": "master", "workflowName": "CI", "displayTitle": "fix: thing",
         "url": "https://github.com/o/r/actions/runs/42"},
        {"databaseId": 43, "status": "in_progress", "conclusion": null,
         "headBranch": "master", "workflowName": "CI", "displayTitle": "wip",
         "url": null},
        {"databaseId": 44, "status": "queued", "conclusion": null,
         "headBranch": null, "workflowName": null, "displayTitle": null, "url": null},
        {"databaseId": 45, "status": "completed", "conclusion": "neutral",
         "headBranch": null, "workflowName": null, "displayTitle": null, "url": null}
    ]"#;
    let runs: Vec<WorkflowRun> = serde_json::from_str(json).unwrap();
    let runs: Vec<BuildRun> = runs.into_iter().map(Into::into).collect();

    assert_eq!(runs[0].status, "success");
    assert!(runs[0].is_terminal());
    assert_eq!(runs[0].id, "42");
    assert_eq!(runs[1].status, "running");
    assert!(!runs[1].is_terminal());
    assert_eq!(runs[2].status, "queued");
    assert_eq!(runs[3].status, "completed");
    assert!(runs[3].is_terminal());
}

#[test]
fn test_pr_check_parsing_and_tally() {
    let json = r#"[
        {"bucket": "pass", "name": "test (ubuntu-latest, stable)", "workflow": "CI",
         "state": "SUCCESS", "startedAt": "2026-09-22T10:00:00Z",
         "completedAt": "2026-09-22T10:05:00Z",
         "link": "https://github.com/o/r/actions/runs/1/job/11",
         "description": "", "event": "pull_request"},
        {"bucket": "fail", "name": "test (macos-latest, stable)", "workflow": "CI",
         "state": "FAILURE", "startedAt": "2026-09-22T10:00:00Z",
         "completedAt": "2026-09-22T10:07:00Z",
         "link": "https://github.com/o/r/actions/runs/1/job/12",
         "description": "", "event": "pull_request"},
        {"bucket": "pending", "name": "clippy", "workflow": "Lint",
         "state": "IN_PROGRESS", "startedAt": "2026-09-22T10:00:00Z",
         "completedAt": null, "link": null, "description": null, "event": null},
        {"bucket": "skipping", "name": "docs", "workflow": "Docs",
         "state": "SKIPPED", "startedAt": null, "completedAt": null,
         "link": null, "description": null, "event": null},
        {"bucket": "pass", "name": "codecov", "workflow": "",
         "state": "SUCCESS", "startedAt": null, "completedAt": null,
         "link": "https://codecov.io/gh/o/r", "description": "92% coverage",
         "event": null}
    ]"#;
    let checks: Vec<PrCheck> = serde_json::from_str(json).unwrap();
    assert_eq!(checks.len(), 5);
    assert_eq!(checks[4].workflow, "", "commit statuses have no workflow");

    let counts = CheckCounts::tally(&checks);
    assert_eq!(counts, CheckCounts { pass: 2, fail: 1, pending: 1, skipped: 1, cancelled: 0 });
    assert_eq!(counts.conclusion(), "failure");
}

#[test]
fn test_check_counts_conclusion_precedence() {
    let success = CheckCounts { pass: 3, ..Default::default() };
    assert_eq!(success.conclusion(), "success");

    let pending = CheckCounts { pass: 3, pending: 1, ..Default::default() };
    assert_eq!(pending.conclusion(), "pending");

    let cancelled = CheckCounts { pass: 3, pending: 1, cancelled: 1, ..Default::default() };
    assert_eq!(cancelled.conclusion(), "cancelled");

    let failed = CheckCounts { pass: 3, pending: 2, cancelled: 1, fail: 1, ..Default::default() };
    assert_eq!(failed.conclusion(), "failure");

    let empty = CheckCounts::default();
    assert_eq!(empty.conclusion(), "no_checks");
    assert_eq!(empty.total(), 0);
}

#[test]
fn test_zero_timestamps_become_null() {
    let json = r#"[
        {"bucket": "pending", "name": "slow", "workflow": "CI", "state": "IN_PROGRESS",
         "startedAt": "2026-09-22T16:35:41Z", "completedAt": "0001-01-01T00:00:00Z",
         "link": null, "description": null, "event": null},
        {"bucket": "pending", "name": "queued", "workflow": "CI", "state": "QUEUED",
         "startedAt": "0001-01-01T00:00:00Z", "completedAt": "0001-01-01T00:00:00Z",
         "link": null, "description": null, "event": null}
    ]"#;
    let checks: Vec<PrCheck> = serde_json::from_str(json).unwrap();

    assert_eq!(checks[0].started_at.as_deref(), Some("2026-09-22T16:35:41Z"));
    assert_eq!(checks[0].completed_at, None);
    assert_eq!(checks[1].started_at, None);
    assert_eq!(checks[1].completed_at, None);
}

#[test]
fn test_watch_termination_conditions() {
    let running = CheckCounts { pass: 2, pending: 2, ..Default::default() };
    assert_eq!(running.pending, 2, "still running: keep polling");

    let all_done = CheckCounts { pass: 4, skipped: 1, ..Default::default() };
    assert_eq!(all_done.pending, 0, "nothing pending: stop");
    assert_eq!(all_done.conclusion(), "success");

    let failing = CheckCounts { pass: 2, fail: 2, pending: 2, ..Default::default() };
    assert!(failing.fail > 0 && failing.pending > 0);
    assert_eq!(failing.conclusion(), "failure");
}

#[test]
fn test_no_checks_is_distinct_from_success() {
    let none = CheckCounts::tally(&[]);
    assert_eq!(none.total(), 0);
    assert_eq!(none.conclusion(), "no_checks");

    let json = r#"[
        {"bucket": "pass", "name": "lint", "workflow": "CI", "state": "SUCCESS",
         "startedAt": null, "completedAt": null, "link": null,
         "description": null, "event": null}
    ]"#;
    let checks: Vec<PrCheck> = serde_json::from_str(json).unwrap();
    assert_eq!(CheckCounts::tally(&checks).conclusion(), "success");
}
