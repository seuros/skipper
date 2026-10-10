mod diff;
mod inspect;
mod mutate;

use crate::git::CommitTrailer;
use crate::git::DiffFormat;
use crate::git::DiffScope;
use crate::git::blame::BlameLine;
use crate::git::file::FileAtRev;
use crate::git::show::ShowEntry;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tempfile::tempdir;

use crate::git::test_support::{git, git_output, init_repo, repo_with_commit};

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

/// `git_diff` of `scope` as `format`: no base, path filter or check.
fn diff_params(scope: DiffScope, format: DiffFormat) -> GitDiffParams {
    GitDiffParams { scope, format, check: false, base: None, paths: None }
}

/// `git_show_file` of `file_path` at HEAD, whole.
fn show_params(file_path: &str) -> GitShowFileParams {
    GitShowFileParams {
        file_path: file_path.to_string(),
        rev: None,
        start_line: None,
        end_line: None,
    }
}

/// `git_commit` of `message`, no trailers.
fn commit(dir: &Path, message: &str, amend: bool) -> Result<serde_json::Value, GitToolError> {
    execute_git_commit_structured(
        dir,
        GitCommitParams { message: message.to_string(), trailers: vec![], amend },
    )
}

fn execute_git_diff_structured(
    cwd: &Path,
    params: GitDiffParams,
) -> Result<serde_json::Value, GitToolError> {
    execute_git_diff_structured_with_cancel(cwd, params, &Arc::new(AtomicBool::new(false)))
}
