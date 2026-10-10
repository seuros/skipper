use super::*;

fn pr(state: &str, draft: bool, mergeable: &str) -> PrOverview {
    PrOverview {
        pr: 6,
        title: "chore: release".into(),
        state: state.into(),
        draft,
        author: "bot".into(),
        head: "release".into(),
        base: "master".into(),
        head_sha: "abc".into(),
        mergeable: mergeable.into(),
        merge_state: "clean".into(),
        review: None,
        merge_methods: vec!["squash", "rebase"],
        default_method: "rebase".into(),
        checks: crate::provider::github::pulls::CheckVerdict::of(Vec::new()),
        labels: vec![],
        additions: 0,
        deletions: 0,
        changed_files: 0,
        merged_at: None,
    }
}

#[test]
fn test_merge_method_defaults_and_refuses_what_github_would() {
    let open = pr("open", false, "mergeable");
    assert_eq!(merge_method(&open, None).unwrap(), "rebase");
    assert_eq!(merge_method(&open, Some("SQUASH")).unwrap(), "squash");

    let refused = |pr: &PrOverview, method: Option<&str>| match merge_method(pr, method) {
        Err(ToolError::InvalidArguments(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert!(refused(&open, Some("merge")).contains("allowed: squash, rebase"));
    assert!(refused(&pr("merged", false, "unknown"), None).contains("is merged"));
    assert!(refused(&pr("open", true, "mergeable"), None).contains("draft"));
    assert!(refused(&pr("open", false, "conflicting"), None).contains("conflicts with master"));
}
