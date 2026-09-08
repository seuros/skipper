use super::*;

#[test]
fn test_router_exposes_the_chosen_git_tools() {
    let names: Vec<String> = router().tools.list().into_iter().map(|t| t.name).collect();

    for tool in [
        "git_status",
        "git_log",
        "git_diff",
        "git_show",
        "git_show_file",
        "git_blame",
        "git_branch",
        "git_add",
        "git_commit",
    ] {
        assert!(names.contains(&tool.to_string()), "missing tool {tool}");
    }

    // git_repo and git_remotes stay internal: the first duplicates repository
    // identity that belongs in a resource, the second hands out raw remote
    // URLs that visibility already encodes.
    assert!(!names.contains(&"git_repo".to_string()));
    assert!(!names.contains(&"git_remotes".to_string()));
    assert_eq!(names.len(), 9);
}
