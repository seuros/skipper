use super::*;

fn pr(number: u64, state: &str, owner: &str) -> BranchPr {
    BranchPr {
        number,
        state: state.to_string(),
        head_repository_owner: Some(Owner { login: owner.to_string() }),
    }
}

fn repo() -> ForgeRepo {
    ForgeRepo {
        remote: "github".into(),
        forge: "github",
        host: "github.com".into(),
        owner: "seuros".into(),
        name: "skipper".into(),
    }
}

#[test]
fn test_branch_pr_prefers_own_open_pr() {
    let prs = [pr(9, "MERGED", "seuros"), pr(8, "OPEN", "fork"), pr(7, "OPEN", "seuros")];
    assert_eq!(pick_branch_pr("seuros", &prs), Some(7));
    assert_eq!(pick_branch_pr("seuros", &[pr(9, "MERGED", "seuros")]), Some(9));
}

#[test]
fn test_branch_pr_refuses_to_guess_between_forks() {
    assert_eq!(pick_branch_pr("seuros", &[pr(4, "OPEN", "alice")]), Some(4));
    assert_eq!(pick_branch_pr("seuros", &[pr(4, "OPEN", "alice"), pr(5, "OPEN", "bob")]), None);
    assert_eq!(pick_branch_pr("seuros", &[]), None);
}

#[test]
fn test_search_query_scopes_to_repo_and_rejects_qualifier_injection() {
    assert_eq!(
        search_query(&repo(), "merged", "me").unwrap(),
        "repo:seuros/skipper is:pr author:@me is:merged sort:created-desc"
    );
    assert_eq!(
        search_query(&repo(), "all", "dependabot[bot]").unwrap(),
        "repo:seuros/skipper is:pr author:dependabot[bot] sort:created-desc"
    );
    assert!(search_query(&repo(), "open", "x repo:other/secret").is_err());
    assert!(search_query(&repo(), "open", "").is_err());
}

#[test]
fn test_overview_lists_allowed_methods_and_cleans_body() {
    let repo: OverviewRepo = serde_json::from_str(
        r#"{"mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":true,
            "viewerDefaultMergeMethod":"SQUASH","pullRequest":null}"#,
    )
    .expect("repo");
    let pull: OverviewPull = serde_json::from_str(
        r#"{"number":6,"title":"chore: release","state":"OPEN","isDraft":false,
            "createdAt":"c","updatedAt":"u","mergedAt":null,
            "body":"Notes<!-- release-please -->\n<details>log</details>","author":null,
            "headRefName":"release","baseRefName":"master","headRefOid":"abc",
            "mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":null,
            "additions":3,"deletions":1,"changedFiles":1,"labels":{"nodes":[{"name":"autorelease: pending"}]},
            "files":{"nodes":[{"path":"Cargo.toml","additions":3,"deletions":1,"changeType":"MODIFIED"}]}}"#,
    )
    .expect("pull");

    let overview = pull.into_overview(&repo);
    assert_eq!(overview.merge_methods, ["squash", "rebase"]);
    assert_eq!(overview.default_method, "squash");
    assert_eq!((overview.state.as_str(), overview.merge_state.as_str()), ("open", "clean"));
    assert_eq!((overview.author.as_str(), overview.body.as_str()), ("ghost", "Notes"));
    assert_eq!(overview.files[0].change, "modified");
}
