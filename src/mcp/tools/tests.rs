use super::*;

#[test]
fn write_confirmation_requires_explicit_approval() {
    use ElicitationAction::{Accept, Cancel, Decline};
    use serde_json::json;

    let summary = "push topic to origin";
    assert!(check_write_confirmation(&Accept, Some(&json!({"approve": true})), summary).is_ok());
    for (action, content, expected) in [
        (Decline, Some(json!({"approve": true})), "approval declined by the client"),
        (Cancel, None, "approval canceled"),
        (Accept, Some(json!({"approve": false})), "approval not granted (approve=false)"),
        (Accept, None, "invalid approval response (expected a boolean `approve` field)"),
        (
            Accept,
            Some(json!({"approve": "true"})),
            "invalid approval response (expected a boolean `approve` field)",
        ),
    ] {
        match check_write_confirmation(&action, content.as_ref(), summary) {
            Err(ToolError::InvalidArguments(message)) => {
                assert_eq!(message, format!("{expected}: {summary}"));
            }
            result => panic!("unexpected confirmation result: {result:?}"),
        }
    }
}

/// Clients reject structured content that breaks the tool's `outputSchema`.
#[track_caller]
fn assert_fits_output_schema(schema: Option<&serde_json::Value>, output: &impl serde::Serialize) {
    let schema = schema.expect("tool declares an outputSchema");
    let output = serde_json::to_value(output).expect("output serializes");
    if let Err(error) = jsonschema::validate(schema, &output) {
        panic!("{error}: {output}");
    }
}

#[test]
fn forge_list_results_are_objects_and_run_lookup_stays_flat() {
    use crate::provider::BuildRun;
    use build_status::BuildStatusResult;
    use serde_json::{json, to_value};

    let run = BuildRun {
        id: "17".into(),
        status: "running",
        branch: None,
        workflow: None,
        title: None,
        url: None,
    };
    let wire_run = to_value(&run).unwrap();
    let schema = SkipperServer::build_status_tool_info().output_schema;
    for (result, expected) in [
        (BuildStatusResult::Run(run.clone()), wire_run.clone()),
        (BuildStatusResult::Recent { runs: vec![run] }, json!({"runs": [wire_run]})),
        (BuildStatusResult::Recent { runs: vec![] }, json!({"runs": []})),
    ] {
        let output = to_value(result).unwrap();
        assert_eq!(output, expected);
        assert_fits_output_schema(schema.as_ref(), &output);
    }
    let repos = gh_repo_list::GhRepoListResult { repos: vec![] };
    assert_eq!(to_value(&repos).unwrap(), json!({"repos": []}));
    assert_fits_output_schema(
        SkipperServer::gh_repo_list_tool_info().output_schema.as_ref(),
        &repos,
    );
}

#[test]
fn test_pr_build_result_reports_only_actionable_checks() {
    use crate::provider::github::PrCheck;
    use pr_build_wait::PrBuildResult;

    let check = |name: &str, bucket: &'static str| PrCheck {
        name: name.to_string(),
        bucket,
        workflow: "CI".to_string(),
        link: Some(format!("https://github.com/o/r/actions/runs/1/job/{name}")),
        description: None,
    };

    let result = PrBuildResult::from_checks(
        vec![
            check("lint", "pass"),
            check("test", "fail"),
            check("bench", "cancel"),
            check("e2e", "pending"),
            check("docs", "skipping"),
        ],
        false,
    );
    assert_eq!(result.conclusion, "failure");
    assert_eq!(result.counts.total(), 5);
    let failed: Vec<&str> = result.failed.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(failed, ["test", "bench"]);
    assert_eq!(result.pending, ["e2e"]);

    let success = PrBuildResult::from_checks(vec![check("lint", "pass")], false);
    let json = serde_json::to_value(&success).unwrap();
    assert!(json.get("failed").is_none() && json.get("pending").is_none());

    let schema = SkipperServer::pr_build_wait_tool_info().output_schema;
    assert_fits_output_schema(schema.as_ref(), &result);
    assert_fits_output_schema(schema.as_ref(), &success);
}

#[test]
fn test_pr_watch_result_fits_output_schema_when_checks_pass() {
    use crate::pr_watch::{ChecksSummary, Event, PrEvent, PrSnapshot};
    use crate::provider::github::CheckCounts;

    let checks = ChecksSummary {
        sha: "abc".to_string(),
        conclusion: "success",
        counts: CheckCounts { pass: 3, ..CheckCounts::default() },
        failed: Vec::new(),
    };
    let result = pr_watch::PrWatchResult {
        pr: 7,
        blocked: true,
        events: vec![PrEvent { pr: 7, event: Arc::new(Event::Checks(checks.clone())) }],
        timed_out: false,
        watching: vec![Arc::new(PrSnapshot {
            pr: 7,
            url: "https://github.com/o/r/pull/7".to_string(),
            state: "open",
            head_sha: "abc".to_string(),
            checks: Some(checks),
        })],
    };

    let schema = SkipperServer::pr_watch_tool_info().output_schema;
    assert_fits_output_schema(schema.as_ref(), &result);
}

#[tokio::test(start_paused = true)]
async fn test_watch_until_outcome_fits_output_schema() {
    use crate::error::Result;
    use crate::provider::BuildRun;
    use crate::provider::github::{DatedRun, OpenPr, PrState};
    use crate::watch_until::{Forge, Step, run};

    struct Merged;
    impl Forge for Merged {
        fn open_prs(&self) -> impl Future<Output = Result<Vec<OpenPr>>> + Send {
            std::future::ready(Ok(Vec::new()))
        }

        fn pr_state(&self, number: u64) -> impl Future<Output = Result<PrState>> + Send {
            std::future::ready(Ok(PrState {
                pr: number,
                title: "release".to_string(),
                head: "release".to_string(),
                state: "merged".to_string(),
                merged_at: Some("2026-11-19T09:00:00Z".to_string()),
            }))
        }

        fn runs(
            &self,
            _branch: Option<&str>,
        ) -> impl Future<Output = Result<Vec<DatedRun>>> + Send {
            std::future::ready(Ok(vec![DatedRun {
                run: BuildRun {
                    id: "9".to_string(),
                    status: "queued",
                    branch: Some("v1".to_string()),
                    workflow: Some("Build".to_string()),
                    title: None,
                    url: None,
                },
                created_at: Some("2026-11-19T09:00:10Z".to_string()),
                head_sha: None,
            }]))
        }
    }

    let steps: Vec<Step> = serde_json::from_value(serde_json::json!([
        { "wait_for": "pr_merged", "pr": 4 },
        { "wait_for": "run_finished", "workflow": "build" },
        { "wait_for": "run_finished", "branch": "v2" }
    ]))
    .unwrap();
    let started = "2026-11-19T08:59:00Z".parse().unwrap();
    let outcome = run(&Merged, &steps, std::time::Duration::from_mins(1), started).await;
    assert!(outcome.expired);

    let info = SkipperServer::watch_until_tool_info();
    assert_fits_output_schema(info.output_schema.as_ref(), &outcome);
    let input = serde_json::to_string(&info.input_schema).unwrap();
    for name in ["pr_opened", "pr_merged", "run_finished", "title_contains"] {
        assert!(input.contains(name), "{name} missing from {input}");
    }
}
