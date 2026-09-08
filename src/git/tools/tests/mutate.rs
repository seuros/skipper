use super::*;
#[test]
fn execute_git_add_and_commit_create_initial_and_followup_commits() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

    let file = dir.join("file.txt");
    fs::write(&file, "alpha\n").expect("write file");

    let added =
        execute_git_add_structured(dir, GitAddParams { paths: vec!["file.txt".to_string()] })
            .expect("add");
    assert_eq!(added["staged"], serde_json::json!(["file.txt"]));
    assert_eq!(added["removed"], serde_json::json!([]));

    let status = crate::git::status(dir).expect("status");
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, "file.txt");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let hook = dir.join(".git/hooks/pre-commit");
        fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hooks dir");
        fs::write(&hook, "#!/bin/sh\necho hook-ran > \"$PWD/hook-ran\"\nexit 1\n")
            .expect("write hook");
        let mut permissions = fs::metadata(&hook).expect("hook metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&hook, permissions).expect("make hook executable");
    }

    let committed = execute_git_commit_structured(
        dir,
        GitCommitParams { message: "initial commit".to_string(), trailers: vec![], amend: false },
    )
    .expect("commit");
    assert_eq!(committed["operation"], "create");
    assert_eq!(committed["previous_sha"], serde_json::Value::Null);
    assert_eq!(committed["subject"], "initial commit");
    assert_eq!(committed["trailers"], serde_json::json!([]));
    assert_eq!(committed["committed_paths"], serde_json::json!(["file.txt"]));
    assert!(!committed["sha"].as_str().unwrap_or_default().is_empty());
    assert_eq!(git_output(dir, &["show", "HEAD:file.txt"]), "alpha\n");
    assert!(!dir.join("hook-ran").exists(), "Git hooks must not run");

    fs::write(&file, "beta\n").expect("modify tracked file");
    fs::write(dir.join("untracked.txt"), "leave me out\n").expect("write untracked file");

    execute_git_add_structured(dir, GitAddParams { paths: vec!["file.txt".to_string()] })
        .expect("stage tracked file");
    execute_git_commit_structured(
        dir,
        GitCommitParams {
            message: "update tracked file".to_string(),
            trailers: vec![],
            amend: false,
        },
    )
    .expect("followup commit");

    assert_eq!(git_output(dir, &["show", "HEAD:file.txt"]), "beta\n");
    assert!(
        git_output(dir, &["status", "--porcelain"]).contains("?? untracked.txt"),
        "unrequested files must remain untracked"
    );

    let empty = execute_git_commit_structured(
        dir,
        GitCommitParams { message: "empty".to_string(), trailers: vec![], amend: false },
    )
    .expect_err("empty commit must fail");
    assert!(empty.contains("nothing staged to commit"));
}

#[test]
fn execute_git_commit_appends_structured_trailers_and_returns_them() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("file.txt"), "alpha\n").expect("write file");
    git(dir, &["add", "file.txt"]);

    let committed = execute_git_commit_structured(
        dir,
        GitCommitParams {
            message: " Implement commit trailers\n\nKeep the API structured. ".to_string(),
            trailers: vec![
                CommitTrailer {
                    token: " Signed-off-by ".to_string(),
                    value: " Mira Tenner <mira-agent@agentmail.to> ".to_string(),
                },
                CommitTrailer {
                    token: "Co-authored-by".to_string(),
                    value: "Daniel Tenner <daniel@tenner.org>".to_string(),
                },
            ],
            amend: false,
        },
    )
    .expect("commit with trailers");

    assert_eq!(committed["operation"], "create");
    assert_eq!(committed["previous_sha"], serde_json::Value::Null);
    assert_eq!(
        committed["trailers"],
        serde_json::json!([
            {
                "token": "Signed-off-by",
                "value": "Mira Tenner <mira-agent@agentmail.to>"
            },
            {
                "token": "Co-authored-by",
                "value": "Daniel Tenner <daniel@tenner.org>"
            }
        ])
    );
    assert_eq!(
        git_output(dir, &["show", "-s", "--format=%B", "HEAD"]),
        concat!(
            "Implement commit trailers\n\n",
            "Keep the API structured.\n\n",
            "Signed-off-by: Mira Tenner <mira-agent@agentmail.to>\n",
            "Co-authored-by: Daniel Tenner <daniel@tenner.org>\n"
        )
    );

    let shown = crate::git::show(dir, None).expect("show committed trailers");
    assert_eq!(
        shown.trailers,
        vec![
            CommitTrailer {
                token: "Signed-off-by".to_string(),
                value: "Mira Tenner <mira-agent@agentmail.to>".to_string(),
            },
            CommitTrailer {
                token: "Co-authored-by".to_string(),
                value: "Daniel Tenner <daniel@tenner.org>".to_string(),
            },
        ]
    );
}

#[test]
fn execute_git_commit_rejects_invalid_structured_trailers() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("file.txt"), "alpha\n").expect("write file");
    git(dir, &["add", "file.txt"]);

    let invalid = [
        (
            CommitTrailer {
                token: "Signed off by".to_string(),
                value: "Test User <test@example.com>".to_string(),
            },
            "invalid commit trailer token",
        ),
        (
            CommitTrailer {
                token: "Signed-off-by".to_string(),
                value: "Test User\nInjected-by: Someone".to_string(),
            },
            "must be a single line",
        ),
        (
            CommitTrailer { token: "Signed-off-by".to_string(), value: "   ".to_string() },
            "must not be empty",
        ),
    ];

    for (trailer, expected) in invalid {
        let error = execute_git_commit_structured(
            dir,
            GitCommitParams {
                message: "invalid trailer".to_string(),
                trailers: vec![trailer],
                amend: false,
            },
        )
        .expect_err("invalid trailer must be rejected");
        assert!(error.contains(expected), "unexpected error: {error}");
    }

    assert_eq!(
        git_output(dir, &["rev-list", "--count", "--all"]),
        "0\n",
        "invalid trailers must not create a commit"
    );
}

#[test]
fn execute_git_add_stages_deletions_and_rejects_unsafe_paths() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

    fs::write(dir.join("tracked.txt"), "tracked\n").expect("write tracked file");
    git(dir, &["add", "tracked.txt"]);
    git(dir, &["commit", "-m", "initial"]);

    fs::remove_file(dir.join("tracked.txt")).expect("remove tracked file");
    let deleted =
        execute_git_add_structured(dir, GitAddParams { paths: vec!["tracked.txt".to_string()] })
            .expect("stage deletion");
    assert_eq!(deleted["removed"], serde_json::json!(["tracked.txt"]));

    fs::write(dir.join(".gitignore"), "*.log\n").expect("write ignore file");
    fs::write(dir.join("ignored.log"), "ignored\n").expect("write ignored file");
    let ignored =
        execute_git_add_structured(dir, GitAddParams { paths: vec!["ignored.log".to_string()] })
            .expect_err("ignored file must fail");
    assert!(ignored.contains("path is ignored: ignored.log"));

    let traversal =
        execute_git_add_structured(dir, GitAddParams { paths: vec!["../outside.txt".to_string()] })
            .expect_err("path traversal must fail");
    assert!(traversal.contains("may not escape the repository"));

    let directory =
        execute_git_add_structured(dir, GitAddParams { paths: vec![".git".to_string()] })
            .expect_err("git directory must fail");
    assert!(directory.contains("Git directory"));
}

#[test]
fn execute_git_add_paths_are_relative_to_repository_root() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    fs::create_dir(dir.join("nested")).expect("create nested directory");
    fs::write(dir.join("root.txt"), "root\n").expect("write root file");

    let added = execute_git_add_structured(
        &dir.join("nested"),
        GitAddParams { paths: vec!["root.txt".to_string()] },
    )
    .expect("stage root-relative path from nested cwd");

    assert_eq!(added["staged"], serde_json::json!(["root.txt"]));
    assert_eq!(git_output(dir, &["diff", "--cached", "--name-only"]), "root.txt\n");
}

#[test]
fn execute_git_commit_preserves_unchanged_gitlinks_and_rejects_changes() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

    fs::write(dir.join("file.txt"), "alpha\n").expect("write file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-m", "initial"]);

    let gitlink_id = git_output(dir, &["rev-parse", "HEAD"]).trim().to_string();
    git(dir, &["update-index", "--add", "--cacheinfo", "160000", &gitlink_id, "vendor/reference"]);
    git(dir, &["commit", "-m", "add gitlink"]);

    fs::write(dir.join("file.txt"), "beta\n").expect("modify file");
    execute_git_add_structured(dir, GitAddParams { paths: vec!["file.txt".to_string()] })
        .expect("stage file");
    execute_git_commit_structured(
        dir,
        GitCommitParams { message: "update file".to_string(), trailers: vec![], amend: false },
    )
    .expect("commit while preserving gitlink");

    assert!(
        git_output(dir, &["ls-tree", "HEAD", "vendor/reference"]).contains(&gitlink_id),
        "unchanged gitlink must be preserved"
    );

    let changed_gitlink_id = git_output(dir, &["rev-parse", "HEAD"]).trim().to_string();
    git(dir, &["update-index", "--cacheinfo", "160000", &changed_gitlink_id, "vendor/reference"]);
    let error = execute_git_commit_structured(
        dir,
        GitCommitParams { message: "change gitlink".to_string(), trailers: vec![], amend: false },
    )
    .expect_err("staged gitlink change must fail");
    assert!(error.contains("staged submodule changes are not supported: vendor/reference"));
}

#[test]
fn execute_git_commit_amends_message_and_includes_staged_changes() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("file.txt"), "alpha\n").expect("write file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-m", "initial"]);
    fs::write(dir.join("second.txt"), "second\n").expect("write second file");
    git(dir, &["add", "second.txt"]);
    git(dir, &["commit", "-m", "second"]);

    let original_sha = git_output(dir, &["rev-parse", "HEAD"]).trim().to_string();
    let original_parent = git_output(dir, &["rev-parse", "HEAD^"]).trim().to_string();
    let original_author = git_output(dir, &["show", "-s", "--format=%an|%ae|%at", "HEAD"]);

    let message_only = execute_git_commit_structured(
        dir,
        GitCommitParams { message: "second, revised".to_string(), trailers: vec![], amend: true },
    )
    .expect("amend message");

    assert_eq!(message_only["operation"], "amend");
    assert_eq!(message_only["previous_sha"].as_str(), Some(original_sha.as_str()));
    assert_eq!(message_only["subject"], "second, revised");
    assert_eq!(message_only["trailers"], serde_json::json!([]));
    assert_eq!(message_only["committed_paths"], serde_json::json!([]));
    assert_ne!(message_only["sha"].as_str().expect("amended sha"), original_sha);
    assert_eq!(git_output(dir, &["rev-parse", "HEAD^"]).trim(), original_parent);
    assert_eq!(
        git_output(dir, &["show", "-s", "--format=%an|%ae|%at", "HEAD"]),
        original_author,
        "amend must preserve the original author"
    );
    assert_eq!(git_output(dir, &["rev-list", "--count", "HEAD"]), "2\n");

    fs::write(dir.join("second.txt"), "updated\n").expect("update second file");
    execute_git_add_structured(dir, GitAddParams { paths: vec!["second.txt".to_string()] })
        .expect("stage amended content");
    let with_changes = execute_git_commit_structured(
        dir,
        GitCommitParams {
            message: "second, revised again".to_string(),
            trailers: vec![],
            amend: true,
        },
    )
    .expect("amend with staged change");

    assert_eq!(with_changes["committed_paths"], serde_json::json!(["second.txt"]));
    assert_eq!(git_output(dir, &["show", "HEAD:second.txt"]), "updated\n");
    assert_eq!(git_output(dir, &["rev-parse", "HEAD^"]).trim(), original_parent);
    assert_eq!(git_output(dir, &["rev-list", "--count", "HEAD"]), "2\n");

    git(dir, &["checkout", "--detach", "HEAD"]);
    let detached = execute_git_commit_structured(
        dir,
        GitCommitParams { message: "detached revision".to_string(), trailers: vec![], amend: true },
    )
    .expect("amend detached HEAD");
    assert_eq!(detached["detached"], true);
    assert_eq!(detached["branch"], serde_json::Value::Null);
    assert_eq!(git_output(dir, &["rev-parse", "--abbrev-ref", "HEAD"]), "HEAD\n");
    assert_eq!(git_output(dir, &["rev-parse", "HEAD^"]).trim(), original_parent);
}

#[test]
fn execute_git_commit_rejects_amend_without_head_commit() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();
    git(dir, &["init"]);

    let error = execute_git_commit_structured(
        dir,
        GitCommitParams { message: "cannot exist".to_string(), trailers: vec![], amend: true },
    )
    .expect_err("unborn HEAD must not be amendable");

    assert!(error.contains("cannot amend because HEAD has no commit"));
}
