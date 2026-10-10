//! Repositories for tests, set up by git itself so skipper reads what git
//! wrote.

use std::fs;
use std::path::Path;
use std::process::Command;

/// `git args` in `dir`; panics with git's stderr when it fails.
pub(crate) fn git(dir: &Path, args: &[&str]) {
    git_output(dir, args);
}

/// `git args` in `dir`, its stdout; panics with git's stderr when it fails.
pub(crate) fn git_output(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(dir).output().expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git output is utf8")
}

/// An empty repo at `dir` on `main`, with an author configured.
pub(crate) fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
}

/// [`init_repo`] plus one commit of `file.txt` holding `content`.
pub(crate) fn repo_with_commit(dir: &Path, content: &str) {
    init_repo(dir);
    fs::write(dir.join("file.txt"), content).expect("write file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-q", "-m", "initial"]);
}
