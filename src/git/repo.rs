use std::path::Path;
use std::path::PathBuf;

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

    let root = repo.workdir().unwrap_or_else(|| repo.git_dir()).to_path_buf();

    let head_sha = repo.head_id().ok().map(|id| id.to_string());

    let branch = repo.head_ref().ok().flatten().map(|r| r.name().shorten().to_string());

    let remote = crate::git::remotes::current_in(&repo);

    let has_changes = repo.is_dirty().unwrap_or(false);

    let default_branch = detect_default_branch(&repo, remote.as_ref().map(|r| r.name.as_str()));

    Ok(RepoInfo { root, head_sha, branch, remote, has_changes, default_branch })
}

/// The current remote's HEAD, else a local `main` or `master`.
fn detect_default_branch(repo: &gix::Repository, remote: Option<&str>) -> Option<String> {
    if let Some(remote_name) = remote {
        let prefix = format!("refs/remotes/{remote_name}/");
        if let Ok(reference) = repo.find_reference(&format!("{prefix}HEAD"))
            && let Some(target) = reference.target().try_name()
            && let Some(branch) = target.as_bstr().to_string().strip_prefix(&prefix)
        {
            return Some(branch.to_owned());
        }
    }

    // Fallback: check for common local defaults
    for candidate in ["main", "master"] {
        let refname = format!("refs/heads/{candidate}");
        if repo.find_reference(&refname).is_ok() {
            return Some(candidate.to_string());
        }
    }

    None
}
