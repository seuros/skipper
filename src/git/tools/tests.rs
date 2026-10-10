mod diff;
mod inspect;
mod mutate;

use crate::git::BlameLine;
use crate::git::CommitTrailer;
use crate::git::DiffFormat;
use crate::git::DiffScope;
use crate::git::FileAtRev;
use crate::git::ShowEntry;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tempfile::tempdir;

use super::GitAddParams;
use super::GitBlameParams;
use super::GitCommitParams;
use super::GitDiffParams;
use super::GitLogParams;
use super::GitShowFileParams;
use super::GitShowParams;
use super::GitToolError;
use super::execute_cancellable_blocking;
use super::execute_git_add_structured;
use super::execute_git_blame_structured;
use super::execute_git_commit_structured;
use super::execute_git_diff_structured_with_cancel;
use super::execute_git_log_structured;
use super::execute_git_show_file_structured;
use super::execute_git_show_structured;

fn execute_git_diff_structured(
    cwd: &Path,
    params: GitDiffParams,
) -> Result<serde_json::Value, GitToolError> {
    execute_git_diff_structured_with_cancel(cwd, params, Arc::new(AtomicBool::new(false)))
}

/// A repo at `dir` with an author configured and one commit of `file.txt`.
fn repo_with_commit(dir: &Path, content: &str) {
    git(dir, &["init"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("file.txt"), content).expect("write file");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-m", "initial"]);
}

fn git(dir: &Path, args: &[&str]) {
    let status =
        Command::new("git").args(args).current_dir(dir).status().expect("failed to run git");
    assert!(status.success(), "git command failed: git {}", args.join(" "));
}

fn git_output(dir: &Path, args: &[&str]) -> String {
    let output =
        Command::new("git").args(args).current_dir(dir).output().expect("failed to run git");
    assert!(
        output.status.success(),
        "git command failed: git {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git output is utf8")
}
