use super::*;

#[test]
fn test_router_registers_every_tool() {
    let router = router();
    let names: Vec<String> = router.tools.list().into_iter().map(|t| t.name).collect();

    #[cfg(feature = "github")]
    for tool in ["gh_repo_list", "pr_build_wait"] {
        assert!(names.contains(&tool.to_string()), "missing tool {tool}");
    }
    #[cfg(feature = "gitlab")]
    assert!(names.contains(&"glab_project_list".to_string()));
    #[cfg(feature = "tea")]
    assert!(names.contains(&"repo_search".to_string()));
    #[cfg(any(feature = "github", feature = "gitlab"))]
    for tool in ["build_status", "build_watch"] {
        assert!(names.contains(&tool.to_string()), "missing tool {tool}");
    }

    #[cfg(feature = "tea")]
    {
        let resources: Vec<String> = router.resources.list().into_iter().map(|r| r.uri).collect();
        assert!(resources.contains(&"skipper://repo".to_string()));
    }

    #[cfg(feature = "github")]
    {
        let templates: Vec<String> =
            router.templates.list().into_iter().map(|t| t.uri_template).collect();
        assert!(templates.contains(&"skipper://pr/{number}/checks".to_string()));
        assert!(templates.contains(&"skipper://pr/{number}/comments".to_string()));
        assert!(templates.contains(&"skipper://pr/{number}/comments/{kind}".to_string()));
        assert!(templates.contains(&"skipper://watch/comments/{kind}".to_string()));
        assert!(templates.contains(&"skipper://prs/{state}/{author}".to_string()));
    }
}

#[cfg(feature = "github")]
#[test]
fn test_pr_build_result_reports_only_actionable_checks() {
    use crate::provider::github::PrCheck;
    use pr_build_wait::PrBuildResult;

    let check = |name: &str, bucket: &str| PrCheck {
        name: name.to_string(),
        bucket: bucket.to_string(),
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
