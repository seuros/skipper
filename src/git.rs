mod add;
mod blame;
mod branch;
mod branches;
mod commit;
mod diff;
mod error;
mod ext;
mod file;
mod log;
mod remotes;
mod repo;
mod show;
mod status;
pub mod tools;

pub use add::AddResult;
pub use add::add;
pub use blame::BlameLine;
pub use blame::blame;
pub use branch::BranchMutationResult;
pub use branch::create as create_branch;
pub use branch::delete as delete_branch;
pub use branches::BranchInfo;
pub use commit::CommitResult;
pub use commit::amend;
pub use commit::amend_with_trailers;
pub use commit::commit;
pub use commit::commit_with_trailers;
pub use diff::DiffFile;
pub use diff::DiffFormat;
pub use diff::DiffReport;
pub use diff::DiffScope;
pub use diff::DiffStatus;
pub use diff::DiffSummary;
pub use diff::WhitespaceError;
pub use diff::diff;
pub use diff::diff_report;
pub use error::GitError;
pub use file::FileAtRev;
pub use file::show_file;
pub use log::LogEntry;
pub use log::log;
pub use remotes::RemoteInfo;
pub use repo::RepoInfo;
pub use show::CommitTrailer;
pub use show::ShowEntry;
pub use show::show;
pub use status::FileStatus;
pub use status::StatusInfo;

use std::path::Path;

fn open_repo(cwd: &Path) -> Result<gix::Repository, GitError> {
    gix::discover(cwd).map_err(|e| GitError::NotARepo(e.to_string()))
}

pub fn repo_info(cwd: &Path) -> Result<RepoInfo, GitError> {
    repo::info(cwd)
}

pub fn repo_root(cwd: &Path) -> Result<std::path::PathBuf, GitError> {
    let repo = open_repo(cwd)?;
    Ok(repo.workdir().unwrap_or_else(|| repo.git_dir()).to_path_buf())
}

pub fn status(cwd: &Path) -> Result<StatusInfo, GitError> {
    status::collect(cwd)
}

pub fn branches(cwd: &Path) -> Result<BranchInfo, GitError> {
    branches::collect(cwd)
}

pub fn remotes(cwd: &Path) -> Result<RemoteInfo, GitError> {
    remotes::collect(cwd)
}

pub struct GitServer;

pub type GitCtx<'a> = mcp_host::macros::Ctx<'a>;
