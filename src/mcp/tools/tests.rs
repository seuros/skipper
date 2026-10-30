use super::*;

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
