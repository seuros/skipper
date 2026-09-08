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

    let resources: Vec<String> = router.resources.list().into_iter().map(|r| r.uri).collect();
    #[cfg(feature = "tea")]
    assert!(resources.contains(&"skipper://repo".to_string()));

    #[cfg(feature = "github")]
    {
        let templates: Vec<String> =
            router.templates.list().into_iter().map(|t| t.uri_template).collect();
        assert!(templates.contains(&"skipper://pr/{number}/checks".to_string()));
    }
}
