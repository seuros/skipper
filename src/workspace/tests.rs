use super::*;

fn repo(remote: &str, owner: &str, name: &str) -> ForgeRepo {
    ForgeRepo {
        remote: remote.to_string(),
        forge: "github",
        host: "github.com".to_string(),
        owner: owner.to_string(),
        name: name.to_string(),
    }
}

#[test]
fn test_pick_on_counts_remotes_of_one_repo_once() {
    let picked =
        pick_on("github", vec![repo("upstream", "me", "app"), repo("github", "me", "app")])
            .expect("one repo");
    assert_eq!(picked.remote, "github");
}

#[test]
fn test_pick_on_refuses_between_repos_and_reports_none() {
    let err = pick_on("github", vec![repo("fork", "me", "app"), repo("upstream", "org", "app")])
        .unwrap_err();
    assert!(
        matches!(err, RemoteError::AmbiguousForge { remotes, .. } if remotes == "fork, upstream")
    );
    assert!(matches!(pick_on("github", Vec::new()), Err(RemoteError::NoForgeRemote("github"))));
}
