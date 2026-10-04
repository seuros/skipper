use std::process::Command;

use tempfile::tempdir;

use super::*;

fn remotes(names: &[&str]) -> BTreeMap<String, String> {
    names.iter().map(|n| ((*n).to_string(), format!("https://example.com/{n}/repo.git"))).collect()
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git").args(args).current_dir(dir).output().expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn test_pick_prefers_upstream_over_origin() {
    let current = pick(Some("github"), remotes(&["github", "origin"])).expect("current");
    assert_eq!((current.name.as_str(), current.source), ("github", RemoteSource::Tracked));
    assert_eq!(current.url, "https://example.com/github/repo.git");
}

#[test]
fn test_pick_falls_back_when_upstream_names_no_remote() {
    let current = pick(Some("."), remotes(&["github", "origin"])).expect("current");
    assert_eq!((current.name.as_str(), current.source), ("origin", RemoteSource::Origin));

    let current = pick(Some("gone"), remotes(&["fork"])).expect("current");
    assert_eq!((current.name.as_str(), current.source), ("fork", RemoteSource::Only));
}

#[test]
fn test_pick_refuses_to_guess_between_remotes() {
    assert_eq!(pick(None, remotes(&["fork", "upstream"])), None);
    assert_eq!(pick(None, remotes(&[])), None);
}

#[test]
fn test_current_follows_branch_upstream_switch() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();
    git(dir, &["init", "-b", "main"]);
    git(dir, &["remote", "add", "origin", "ssh://git@forge.local:2222/me/repo.git"]);
    git(dir, &["remote", "add", "github", "https://github.com/me/repo.git"]);

    let current = super::current(dir).expect("read").expect("current");
    assert_eq!((current.name.as_str(), current.source), ("origin", RemoteSource::Origin));

    git(dir, &["config", "branch.main.remote", "github"]);
    let current = super::current(dir).expect("read").expect("current");
    assert_eq!((current.name.as_str(), current.source), ("github", RemoteSource::Tracked));
}
