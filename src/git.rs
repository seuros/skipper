mod add;
mod blame;
mod branch;
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
mod sync;
pub mod tools;

pub use add::AddResult;
pub use add::add;
pub use blame::BlameLine;
pub use blame::blame;
pub use branch::BranchMutationResult;
pub use branch::create as create_branch;
pub use branch::delete as delete_branch;
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
pub use remotes::CurrentRemote;
pub use remotes::RemoteInfo;
pub use remotes::RemoteSource;
pub use repo::RepoInfo;
pub use show::CommitTrailer;
pub use show::ShowEntry;
pub use show::commit_id;
pub use show::show;
pub use status::FileStatus;
pub use status::StatusCounts;
pub use status::StatusInfo;
pub use sync::PushPlan;
pub use sync::SyncOutcome;
pub use sync::fetch;
pub use sync::plan_push;
pub use sync::pull;
pub use sync::push;

use std::path::Path;

fn open_repo(cwd: &Path) -> Result<gix::Repository, GitError> {
    gix::discover(cwd).map_err(|e| GitError::NotARepo(e.to_string()))
}

pub fn repo_info(cwd: &Path) -> Result<RepoInfo, GitError> {
    repo::info(cwd)
}

/// [`repo_info`] and every remote, from one open of the repository.
pub fn repo_info_with_remotes(
    cwd: &Path,
) -> Result<(RepoInfo, std::collections::BTreeMap<String, String>), GitError> {
    repo::info_with_remotes(cwd)
}

pub fn status(cwd: &Path) -> Result<StatusInfo, GitError> {
    status::collect(cwd)
}

pub fn status_counts(cwd: &Path) -> Result<StatusCounts, GitError> {
    status::counts(cwd)
}

pub fn is_clean(cwd: &Path) -> Result<bool, GitError> {
    status::is_clean(cwd)
}

pub fn has_staged(cwd: &Path) -> Result<bool, GitError> {
    status::has_staged(cwd)
}

/// The checked-out branch; `None` on a detached HEAD. Reads HEAD alone,
/// where [`repo_info`] also walks the worktree and the remotes.
pub fn current_branch(cwd: &Path) -> Result<Option<String>, GitError> {
    Ok(repo::head_branch(&open_repo(cwd)?))
}

pub fn remotes(cwd: &Path) -> Result<RemoteInfo, GitError> {
    remotes::collect(cwd)
}

/// The remote forge reads go to: the branch's upstream, else the only remote,
/// else `origin`. `None` when several remotes leave it open.
pub fn current_remote(cwd: &Path) -> Result<Option<CurrentRemote>, GitError> {
    remotes::current(cwd)
}

/// [`remotes`] and [`current_remote`] from one open of the repository.
pub fn remotes_with_current(cwd: &Path) -> Result<(RemoteInfo, Option<CurrentRemote>), GitError> {
    remotes::with_current(cwd)
}

pub struct GitServer;

pub type GitCtx<'a> = mcp_host::macros::Ctx<'a>;
