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
        {"id": 42, "status": "completed", "conclusion": "success", "head_branch": "master",
         "name": "CI", "display_title": "fix: thing",
         "html_url": "https://github.com/o/r/actions/runs/42"},
        {"id": 43, "status": "in_progress", "conclusion": null, "head_branch": "master",
         "name": "CI", "display_title": "wip", "html_url": null},
        {"id": 44, "status": "queued", "conclusion": null, "head_branch": null,
         "name": null, "display_title": null, "html_url": null},
        {"id": 45, "status": "completed", "conclusion": "neutral", "head_branch": null,
         "name": null, "display_title": null, "html_url": null}
    ]"#;
    let runs: Vec<runs::WorkflowRun> = serde_json::from_str(json).unwrap();
    let runs: Vec<BuildRun> = runs.into_iter().map(Into::into).collect();

    assert_eq!((runs[0].status.as_str(), runs[0].id.as_str()), ("success", "42"));
    assert_eq!(runs[0].workflow.as_deref(), Some("CI"));
    assert!(runs[0].is_terminal());
    assert_eq!(runs[1].status, "running");
    assert!(!runs[1].is_terminal());
    assert_eq!(runs[2].status, "queued");
    assert_eq!(runs[3].status, "completed");
    assert!(runs[3].is_terminal());
}

#[test]
fn test_check_tally() {
    let checks = [
        check("test (ubuntu)", "pass"),
        check("test (macos)", "fail"),
        check("clippy", "pending"),
        check("docs", "skipping"),
        check("codecov", "pass"),
    ];
    let counts = CheckCounts::tally(&checks);
    assert_eq!(counts, CheckCounts { pass: 2, fail: 1, pending: 1, skipped: 1, cancelled: 0 });
    assert_eq!(counts.conclusion(), "failure");
}

#[test]
fn test_buckets_match_gh_for_rest_and_graphql_values() {
    assert_eq!(run_bucket("completed", Some("success")), "pass");
    assert_eq!(run_bucket("COMPLETED", Some("NEUTRAL")), "skipping");
    assert_eq!(run_bucket("COMPLETED", Some("SKIPPED")), "skipping");
    assert_eq!(run_bucket("completed", Some("cancelled")), "cancel");
    assert_eq!(run_bucket("COMPLETED", Some("STARTUP_FAILURE")), "fail");
    assert_eq!(run_bucket("completed", Some("action_required")), "fail");
    assert_eq!(run_bucket("COMPLETED", Some("STALE")), "pending");
    assert_eq!(run_bucket("IN_PROGRESS", None), "pending");
    assert_eq!(status_bucket("SUCCESS"), "pass");
    assert_eq!(status_bucket("expected"), "pending");
    assert_eq!(status_bucket("ERROR"), "fail");
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

    assert_eq!(CheckCounts::tally(&[check("lint", "pass")]).conclusion(), "success");
}

#[test]
fn test_auth_network_failure_is_unknown_not_logged_out() {
    let host = |entry: &str| {
        let json = format!(r#"{{"hosts":{{"github.com":[{entry}]}}}}"#);
        serde_json::from_str::<AuthReport>(&json).unwrap().logged_in()
    };
    assert!(host(r#"{"state":"success"}"#).unwrap());
    assert!(!host(r#"{"state":"error","error":"HTTP 401: Bad credentials"}"#).unwrap());
    assert!(host(r#"{"state":"timeout","error":"i/o timeout"}"#).is_err());
    assert!(host(r#"{"state":"error","error":"read: connection reset by peer"}"#).is_err());
}

#[test]
fn test_retryable_is_network_and_server_errors_only() {
    assert!(retryable(&CliError::io("github", std::io::Error::other("connection refused"))));
    assert!(retryable(&CliError::execution_failed("github", 502, "GET x: HTTP 502")));
    assert!(!retryable(&CliError::execution_failed("github", 404, "GET x: Not Found")));
    assert!(!retryable(&CliError::execution_failed(
        "github",
        200,
        "Could not resolve to a PullRequest"
    )));
}

#[tokio::test(start_paused = true)]
async fn test_retrying_retries_network_failures_only() {
    let calls = std::cell::Cell::new(0u32);
    let value = retrying(|| {
        calls.set(calls.get() + 1);
        let n = calls.get();
        async move {
            if n < 3 {
                Err(CliError::execution_failed("gh", 1, "read: connection reset by peer"))
            } else {
                Ok(n)
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(value, 3);

    let calls = std::cell::Cell::new(0u32);
    let failed: Result<u32> = retrying(|| {
        calls.set(calls.get() + 1);
        async { Err(CliError::execution_failed("gh", 1, "HTTP 404: Not Found")) }
    })
    .await;
    assert!(failed.is_err());
    assert_eq!(calls.get(), 1);

    let calls = std::cell::Cell::new(0u32);
    let exhausted: Result<u32> = retrying(|| {
        calls.set(calls.get() + 1);
        async { Err(CliError::execution_failed("gh", 1, "read: connection reset by peer")) }
    })
    .await;
    assert!(exhausted.unwrap_err().to_string().contains("connection reset"));
    assert_eq!(calls.get(), 4);
}

fn check(name: &str, bucket: &str) -> PrCheck {
    PrCheck {
        name: name.to_string(),
        bucket: bucket.to_string(),
        workflow: "CI".to_string(),
        link: None,
        description: None,
    }
}

fn reset() -> CliError {
    CliError::execution_failed(
        "gh",
        1,
        r#"Post "https://api.github.com/graphql": read tcp 10.0.0.2:55499->140.82.121.6:443: read: connection reset by peer"#,
    )
}

type Step = fn() -> Result<Vec<PrCheck>>;
type Calls = std::rc::Rc<std::cell::Cell<usize>>;
type Polled = std::future::Ready<Result<Vec<PrCheck>>>;

/// Poll results in order; the last one repeats once the script runs out.
fn scripted(script: Vec<Step>) -> (Calls, impl FnMut() -> Polled) {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = calls.clone();
    let poll = move || {
        let n = counter.get();
        counter.set(n + 1);
        std::future::ready(script[n.min(script.len() - 1)]())
    };
    (calls, poll)
}

const MINUTE: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(10);

#[tokio::test(start_paused = true)]
async fn test_watch_rides_out_network_failures() {
    let (calls, poll) = scripted(vec![
        || Err(reset()),
        || Err(CliError::timeout("gh", Duration::from_secs(30))),
        || Ok(vec![check("build", "pass"), check("test", "pending")]),
        || {
            Err(CliError::execution_failed(
                "gh",
                1,
                r#"Post "https://api.github.com/graphql": dial tcp 140.82.121.6:443: connect: operation timed out"#,
            ))
        },
        || Ok(vec![check("build", "pass"), check("test", "pass")]),
    ]);
    let watch = watch_checks(poll, true, 10 * MINUTE, TICK).await.unwrap();
    assert!(!watch.timed_out, "a call's own timeout is not the watch deadline");
    assert_eq!(CheckCounts::tally(&watch.checks).conclusion(), "success");
    assert_eq!(calls.get(), 5);
}

#[tokio::test(start_paused = true)]
async fn test_watch_deadline_reports_last_snapshot() {
    let (_, poll) =
        scripted(vec![|| Ok(vec![check("build", "pass"), check("test", "pending")]), || {
            Err(reset())
        }]);
    let watch = watch_checks(poll, true, MINUTE, TICK).await.unwrap();
    assert!(watch.timed_out);
    assert_eq!(CheckCounts::tally(&watch.checks).pending, 1);
}

#[tokio::test(start_paused = true)]
async fn test_watch_fails_when_no_poll_gets_through() {
    let (calls, poll) = scripted(vec![|| Err(reset())]);
    let error = watch_checks(poll, true, MINUTE, TICK).await.unwrap_err();
    assert!(error.to_string().contains("connection reset"));
    assert!(calls.get() > 1, "network failures are retried until the deadline");
}

#[tokio::test(start_paused = true)]
async fn test_watch_stops_at_once_on_other_errors() {
    let (calls, poll) = scripted(vec![|| {
        Err(CliError::execution_failed("gh", 1, "no pull requests found for branch \"x\""))
    }]);
    assert!(watch_checks(poll, true, MINUTE, TICK).await.is_err());
    assert_eq!(calls.get(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_watch_fail_fast() {
    let script: Vec<fn() -> Result<Vec<PrCheck>>> =
        vec![|| Ok(vec![check("build", "fail"), check("test", "pending")]), || {
            Ok(vec![check("build", "fail"), check("test", "pass")])
        }];

    let (calls, poll) = scripted(script.clone());
    let watch = watch_checks(poll, true, MINUTE, TICK).await.unwrap();
    assert_eq!(calls.get(), 1, "fail_fast returns on the first failure");
    assert_eq!(CheckCounts::tally(&watch.checks).pending, 1);

    let (calls, poll) = scripted(script);
    let watch = watch_checks(poll, false, MINUTE, TICK).await.unwrap();
    assert_eq!(calls.get(), 2, "without fail_fast the watch waits for every check");
    assert_eq!(CheckCounts::tally(&watch.checks).pending, 0);
}
