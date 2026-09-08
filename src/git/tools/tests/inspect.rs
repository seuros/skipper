use super::*;
#[test]
fn execute_git_blame_includes_head_author_placeholder() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

    let file = dir.join("file.txt");
    fs::write(&file, "alpha\nbeta\n").expect("write file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-m", "initial"]);

    let blame_json = execute_git_blame_structured(
        dir,
        GitBlameParams {
            file_path: "file.txt".to_string(),
            start_line: Some(1),
            end_line: Some(1),
        },
    )
    .expect("blame");

    let blamed: Vec<BlameLine> =
        serde_json::from_value(blame_json["lines"].clone()).expect("parse blame json");
    assert_eq!(blamed.len(), 1);
    assert_eq!(blamed[0].author, "Test User");
    assert_eq!(blamed[0].content, "alpha");
    assert!(!blamed[0].sha.is_empty());
}

#[test]
fn execute_git_show_returns_subject_body_and_trailers() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path();

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

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

    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);

    fs::create_dir(dir.join("src")).expect("create src");
    fs::write(dir.join("src/file.txt"), "alpha\nbeta\ngamma\n").expect("write file");
    git(dir, &["add", "src/file.txt"]);
    git(dir, &["commit", "-m", "initial"]);
    let head = git_output(dir, &["rev-parse", "HEAD"]).trim().to_string();

    fs::write(dir.join("src/file.txt"), "worktree\n").expect("dirty worktree");

    let shown_json = execute_git_show_file_structured(
        dir,
        GitShowFileParams {
            file_path: "src/file.txt".to_string(),
            rev: None,
            start_line: None,
            end_line: None,
        },
    )
    .expect("show file");
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
            file_path: "./src/file.txt".to_string(),
            rev: Some("HEAD".to_string()),
            start_line: Some(2),
            end_line: Some(3),
        },
    )
    .expect("show file range");
    assert_eq!(ranged["content"], "beta\ngamma\n");
    assert_eq!(ranged["start_line"], 2);
    assert_eq!(ranged["end_line"], 3);
    assert_eq!(ranged["total_lines"], 3);

    let missing = execute_git_show_file_structured(
        dir,
        GitShowFileParams {
            file_path: "missing.txt".to_string(),
            rev: None,
            start_line: None,
            end_line: None,
        },
    )
    .expect_err("missing file must fail");
    assert!(missing.contains("path not found: missing.txt"));

    let incomplete = execute_git_show_file_structured(
        dir,
        GitShowFileParams {
            file_path: "src/file.txt".to_string(),
            rev: None,
            start_line: Some(1),
            end_line: None,
        },
    )
    .expect_err("partial line range must fail");
    assert!(incomplete.contains("both be provided or both be omitted"));

    let directory = execute_git_show_file_structured(
        dir,
        GitShowFileParams {
            file_path: "src".to_string(),
            rev: None,
            start_line: None,
            end_line: None,
        },
    )
    .expect_err("directory must fail");
    assert!(directory.contains("path is a directory: src"));
}
