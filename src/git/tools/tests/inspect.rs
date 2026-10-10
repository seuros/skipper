use super::*;

#[test]
fn execute_git_log_sizes_by_history_not_by_limit() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();
    repo_with_commit(dir, "alpha\n");

    // A limit no history reaches must not be allocated up front.
    let log = execute_git_log_structured(dir, GitLogParams { limit: usize::MAX, branch: None })
        .expect("log");
    assert_eq!(log["commits"].as_array().map(Vec::len), Some(1), "{log}");
}

#[test]
fn execute_git_blame_attributes_each_line_to_its_commit() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();
    repo_with_commit(dir, "alpha\nbeta\ngamma\n");
    fs::write(dir.join("file.txt"), "alpha\nBETA\ngamma\n").expect("write file");
    git(dir, &["-c", "user.name=Second Author", "commit", "-q", "-am", "second"]);
    let short = |rev: &str| git_output(dir, &["rev-parse", "--short=8", rev]).trim().to_owned();
    let (first, second) = (short("HEAD~1"), short("HEAD"));

    let blame = |start_line, end_line| -> Vec<BlameLine> {
        let params = GitBlameParams { file_path: "file.txt".to_string(), start_line, end_line };
        let json = execute_git_blame_structured(dir, params).expect("blame");
        serde_json::from_value(json["lines"].clone()).expect("parse blame json")
    };
    let owners = |lines: &[BlameLine]| {
        lines
            .iter()
            .map(|l| (l.line_no, l.content.clone(), l.author.to_string(), l.sha.to_string()))
            .collect::<Vec<_>>()
    };
    let line = |n, content: &str, author: &str, sha: &str| {
        (n, content.to_owned(), author.to_owned(), sha.to_owned())
    };

    assert_eq!(
        owners(&blame(None, None)),
        [
            line(1, "alpha", "Test User", &first),
            line(2, "BETA", "Second Author", &second),
            line(3, "gamma", "Test User", &first),
        ]
    );
    // A range blames those lines only; past the end there is nothing.
    assert_eq!(owners(&blame(Some(2), Some(3)))[0], line(2, "BETA", "Second Author", &second));
    assert_eq!(blame(Some(2), Some(3)).len(), 2);
    assert!(blame(Some(10), Some(12)).is_empty());
}

#[test]
fn execute_git_show_returns_subject_body_and_trailers() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    init_repo(dir);

    let file = dir.join("file.txt");
    fs::write(&file, "alpha\n").expect("write file");
    git(dir, &["add", "file.txt"]);
    git(
        dir,
        &[
            "commit",
            "-m",
            "feat: roast engine online",
            "-m",
            "Claude wrote a commit body with all the charisma of a tax form.\n\nSigned-off-by: Test User <test@example.com>",
        ],
    );

    let show_json =
        execute_git_show_structured(dir, GitShowParams { rev: Some("HEAD".to_string()) })
            .expect("show");

    let shown: ShowEntry = serde_json::from_value(show_json).expect("parse show json");
    assert_eq!(shown.subject, "feat: roast engine online");
    assert!(shown.body.contains("charisma of a tax form"));
    assert_eq!(shown.author, "Test User");
    assert_eq!(shown.trailers.len(), 1);
    assert_eq!(shown.trailers[0].token, "Signed-off-by");
    assert_eq!(shown.trailers[0].value, "Test User <test@example.com>");
}

#[test]
fn execute_git_show_file_reads_revision_not_worktree() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    init_repo(dir);

    fs::create_dir(dir.join("src")).expect("create src");
    fs::write(dir.join("src/file.txt"), "alpha\nbeta\ngamma\n").expect("write file");
    git(dir, &["add", "src/file.txt"]);
    git(dir, &["commit", "-m", "initial"]);
    let head = git_output(dir, &["rev-parse", "HEAD"]).trim().to_string();

    fs::write(dir.join("src/file.txt"), "worktree\n").expect("dirty worktree");

    let shown_json =
        execute_git_show_file_structured(dir, show_params("src/file.txt")).expect("show file");
    let shown: FileAtRev = serde_json::from_value(shown_json).expect("parse show file json");
    assert_eq!(shown.path, "src/file.txt");
    assert_eq!(shown.rev, "HEAD");
    assert_eq!(shown.sha, head);
    assert_eq!(shown.content, "alpha\nbeta\ngamma\n");
    assert_eq!(shown.total_lines, 3);
    assert_eq!(shown.start_line, None);
    assert_eq!(shown.end_line, None);

    let ranged = execute_git_show_file_structured(
        dir,
        GitShowFileParams {
            rev: Some("HEAD".to_string()),
            start_line: Some(2),
            end_line: Some(3),
            ..show_params("./src/file.txt")
        },
    )
    .expect("show file range");
    assert_eq!(ranged["content"], "beta\ngamma\n");
    assert_eq!(ranged["start_line"], 2);
    assert_eq!(ranged["end_line"], 3);
    assert_eq!(ranged["total_lines"], 3);

    let missing = execute_git_show_file_structured(dir, show_params("missing.txt"))
        .expect_err("missing file must fail");
    assert!(missing.to_string().contains("path not found: missing.txt"));

    let unknown = execute_git_show_file_structured(
        dir,
        GitShowFileParams { rev: Some("no-such-rev".to_string()), ..show_params("src/file.txt") },
    )
    .expect_err("unknown revision must fail");
    assert!(matches!(unknown, GitToolError::Caller(_)), "{unknown}");
    assert!(!unknown.to_string().contains(".rs:"), "source location leaked: {unknown}");

    let incomplete = execute_git_show_file_structured(
        dir,
        GitShowFileParams { start_line: Some(1), ..show_params("src/file.txt") },
    )
    .expect_err("partial line range must fail");
    assert!(incomplete.to_string().contains("both be provided or both be omitted"));

    let directory =
        execute_git_show_file_structured(dir, show_params("src")).expect_err("directory must fail");
    assert!(directory.to_string().contains("path is a directory: src"));
}
