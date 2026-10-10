use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::git::error::GitError;
use crate::git::open_repo;
use crate::git::remotes::CurrentRemote;

#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub head_sha: Option<String>,
    pub branch: Option<String>,
    pub remote: Option<CurrentRemote>,
    pub has_changes: bool,
    pub default_branch: Option<String>,
}

pub fn info(cwd: &Path) -> Result<RepoInfo, GitError> {
    let repo = open_repo(cwd)?;
    let remote = crate::git::remotes::current_in(&repo);
    Ok(info_in(&repo, remote))
}

/// [`info`] and every remote, from one open of the repository.
pub fn info_with_remotes(cwd: &Path) -> Result<(RepoInfo, BTreeMap<String, String>), GitError> {
    let repo = open_repo(cwd)?;
    let (remotes, current) = crate::git::remotes::with_current_in(&repo);
    Ok((info_in(&repo, current), remotes))
}

fn info_in(repo: &gix::Repository, remote: Option<CurrentRemote>) -> RepoInfo {
    let root = repo.workdir().unwrap_or_else(|| repo.git_dir()).to_path_buf();

    let head_sha = repo.head_id().ok().map(|id| id.to_string());

    let branch = head_branch(repo);

    let has_changes = repo.is_dirty().unwrap_or(false);

    let default_branch = detect_default_branch(repo, remote.as_ref().map(|r| r.name.as_str()));

    RepoInfo { root, head_sha, branch, remote, has_changes, default_branch }
}

/// The checked-out branch's short name; `None` on a detached HEAD.
pub(super) fn head_branch(repo: &gix::Repository) -> Option<String> {
    repo.head_ref().ok().flatten().map(|r| r.name().shorten().to_string())
}

/// The current remote's HEAD, else a local `main` or `master`.
fn detect_default_branch(repo: &gix::Repository, remote: Option<&str>) -> Option<String> {
    if let Some(remote_name) = remote {
        let head = format!("refs/remotes/{remote_name}/HEAD");
        let prefix = &head.as_bytes()[..head.len() - "HEAD".len()];
        if let Ok(reference) = repo.find_reference(head.as_str())
            && let Some(target) = reference.target().try_name()
            && let Some(branch) = target.as_bstr().strip_prefix(prefix)
        {
            return Some(branch.to_str_lossy().into_owned());
        }
    }

    // Fallback: check for common local defaults
    for (candidate, refname) in [("main", "refs/heads/main"), ("master", "refs/heads/master")] {
        if repo.find_reference(refname).is_ok() {
            return Some(candidate.to_owned());
        }
    }

    None
}
