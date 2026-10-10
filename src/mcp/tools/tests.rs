use super::*;

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
}
