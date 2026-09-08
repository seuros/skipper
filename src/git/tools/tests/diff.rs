use super::*;
#[test]
fn execute_git_diff_returns_scoped_structured_formats_and_checks() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

    let file = dir.join("file.txt");
    fs::write(&file, "one\ntwo\n").expect("write initial file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-m", "initial"]);

    fs::write(&file, "one\nstaged\n").expect("write staged file");
    git(dir, &["add", "file.txt"]);
    fs::write(&file, "one\nworktree  \n").expect("write worktree file");

    let staged = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::Staged,
            format: DiffFormat::Patch,
            check: false,
            base: None,
            paths: Some(vec!["file.txt".to_string()]),
        },
    )
    .expect("staged patch");
    let staged_patch = staged["result"]["files"][0]["patch"].as_str().expect("staged patch text");
    assert!(staged_patch.contains("--- a/file.txt"));
    assert!(staged_patch.contains("+++ b/file.txt"));
    assert!(staged_patch.contains("-two"));
    assert!(staged_patch.contains("+staged"));
    assert!(!staged_patch.contains("worktree"));

    let worktree = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::Worktree,
            format: DiffFormat::Stat,
            check: false,
            base: None,
            paths: None,
        },
    )
    .expect("worktree stat");
    assert_eq!(worktree["scope"], "worktree");
    assert_eq!(worktree["summary"]["files_changed"], 1);
    assert_eq!(worktree["result"]["format"], "stat");
    assert_eq!(worktree["result"]["files"][0]["path"], "file.txt");
    assert_eq!(worktree["result"]["files"][0]["additions"], 1);
    assert_eq!(worktree["result"]["files"][0]["deletions"], 1);

    let all = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::All,
            format: DiffFormat::NameOnly,
            check: true,
            base: None,
            paths: None,
        },
    )
    .expect("all changed paths");
    assert_eq!(all["base"], "HEAD");
    assert_eq!(all["result"]["format"], "name_only");
    assert_eq!(all["result"]["paths"], serde_json::json!(["file.txt"]));
    assert_eq!(all["whitespace_check"]["passed"], false);
    assert_eq!(all["whitespace_check"]["errors"][0]["kind"], "trailing_whitespace");
    assert_eq!(all["whitespace_check"]["errors"][0]["line"], 2);

    let error = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::Worktree,
            format: DiffFormat::NameOnly,
            check: false,
            base: Some("HEAD".to_string()),
            paths: None,
        },
    )
    .expect_err("worktree base must be rejected");
    assert!(error.contains("base cannot be used with worktree scope"));
}

#[test]
fn execute_git_diff_name_only_skips_oversized_blob_content() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    let large = vec![b'x'; 9 * 1024 * 1024];
    fs::write(dir.join("generated.txt"), large).expect("write large file");
    git(dir, &["add", "generated.txt"]);

    let names = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::Staged,
            format: DiffFormat::NameOnly,
            check: false,
            base: None,
            paths: None,
        },
    )
    .expect("name-only should not load blob content");
    assert_eq!(names["result"]["paths"], serde_json::json!(["generated.txt"]));
    assert!(!names["summary"].as_object().expect("summary object").contains_key("insertions"));

    let error = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::Staged,
            format: DiffFormat::Patch,
            check: false,
            base: None,
            paths: None,
        },
    )
    .expect_err("patch generation must reject oversized content");
    assert!(error.contains("per-file limit"));
    assert!(error.contains("generated.txt"));
}

#[test]
fn execute_git_diff_all_filters_staged_changes_undone_in_worktree() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("file.txt"), "head\n").expect("write initial file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-m", "initial"]);

    fs::write(dir.join("file.txt"), "staged\n").expect("write staged content");
    git(dir, &["add", "file.txt"]);
    fs::write(dir.join("file.txt"), "head\n").expect("restore head content");

    let all = execute_git_diff_structured(
        dir,
        GitDiffParams {
            scope: DiffScope::All,
            format: DiffFormat::NameOnly,
            check: false,
            base: None,
            paths: None,
        },
    )
    .expect("all diff");
    assert_eq!(all["result"]["paths"], serde_json::json!([]));
    assert_eq!(all["summary"]["files_changed"], 0);
}

#[test]
fn cancellable_blocking_waits_for_worker_shutdown_after_timeout() {
    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);

    let result = runtime.block_on(execute_cancellable_blocking(
        PathBuf::from("."),
        (),
        Duration::from_millis(20),
        "test operation",
        move |_, (), cancel| {
            while !cancel.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            worker_stopped.store(true, Ordering::Release);
            Ok::<(), String>(())
        },
    ));

    assert_eq!(result.expect_err("operation must time out"), "test operation timed out after 20ms");
    assert!(stopped.load(Ordering::Acquire), "timeout must not leave blocking work running");
}
