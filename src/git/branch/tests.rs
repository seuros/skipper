use std::fs;
use std::process::Command;

use tempfile::tempdir;

use super::*;
use crate::git::test_support::{git, repo_with_commit};

/// Whether `refs/heads/{name}` exists, as git itself sees it.
fn has_branch(dir: &Path, name: &str) -> bool {
    Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{name}")])
        .current_dir(dir)
        .status()
        .expect("run git")
        .success()
}

#[test]
fn create_does_not_checkout_and_delete_removes_merged_branch() {
    let temp = tempdir().expect("tempdir");
    repo_with_commit(temp.path(), "initial\n");

    let created = create(temp.path(), "topic", None).expect("create branch");
    assert_eq!(created.operation, "create");
    assert_eq!(crate::git::current_branch(temp.path()).expect("head").as_deref(), Some("main"));
    assert!(has_branch(temp.path(), "topic"));

    let deleted = delete(temp.path(), "topic", false).expect("delete merged branch");
    assert_eq!(deleted.oid, created.oid);
    assert!(!has_branch(temp.path(), "topic"));
}

#[test]
fn delete_rejects_unmerged_branch_without_force() {
    let temp = tempdir().expect("tempdir");
    repo_with_commit(temp.path(), "initial\n");
    git(temp.path(), &["switch", "-c", "topic"]);
    fs::write(temp.path().join("topic.txt"), "topic\n").expect("write topic");
    git(temp.path(), &["add", "topic.txt"]);
    git(temp.path(), &["commit", "-m", "topic"]);
    git(temp.path(), &["switch", "main"]);

    let error = delete(temp.path(), "topic", false).expect_err("reject unmerged");
    assert!(error.to_string().contains("not fully merged"));
    delete(temp.path(), "topic", true).expect("force delete");
}

#[test]
fn delete_rejects_branch_checked_out_in_linked_worktree() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("worktree");
    fs::create_dir(&repo).expect("create repo");
    repo_with_commit(&repo, "initial\n");
    create(&repo, "topic", None).expect("create topic");
    git(&repo, &["worktree", "add", worktree.to_str().expect("utf8 worktree"), "topic"]);

    let error = delete(&repo, "topic", true).expect_err("reject checked out branch");
    assert!(error.to_string().contains("checked out"));
}

#[test]
fn rejects_full_reference_names() {
    let temp = tempdir().expect("tempdir");
    repo_with_commit(temp.path(), "initial\n");
    let error = create(temp.path(), "refs/heads/topic", None).expect_err("reject full name");
    assert!(error.to_string().contains("short local name"));
}
