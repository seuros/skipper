use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;

use crate::environment::{Environment as _, SkipperEnvironment};
use crate::git::GitError;

/// The current workspace: the working directory and,
/// when that sits inside a repository, the repository's identity.
#[derive(Debug, Clone, Serialize)]
pub struct Workspace {
    pub cwd: PathBuf,
    pub repo: Option<WorkspaceRepo>,
    /// Forges the remotes resolve to; forge tools are scoped to these.
    pub forges: BTreeSet<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRepo {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub default_branch: Option<String>,
    pub dirty: bool,
    /// `origin` first, the rest by name.
    pub remotes: Vec<WorkspaceRemote>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRemote {
    pub name: String,
    /// Fetch URL with embedded credentials stripped.
    pub url: String,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub forge: Option<&'static str>,
}

pub fn snapshot(env: &SkipperEnvironment) -> Result<Workspace, GitError> {
    let cwd = env.cwd().to_path_buf();

    let info = match crate::git::repo_info(&cwd) {
        Ok(info) => info,
        Err(GitError::NotARepo(_)) => {
            return Ok(Workspace { cwd, repo: None, forges: BTreeSet::new() });
        }
        Err(e) => return Err(e),
    };

    let remotes: Vec<WorkspaceRemote> =
        crate::remote::ordered_remotes(crate::git::remotes(&cwd)?.remotes)
            .into_iter()
            .map(|(name, url)| {
                let host = crate::remote::host_of(&url);
                let (owner, repo) =
                    host.as_ref().and_then(|_| crate::remote::repo_path_of(&url)).unzip();
                WorkspaceRemote {
                    forge: env.forge_for_url(&url),
                    url: crate::remote::redact_url(&url),
                    name,
                    host,
                    owner,
                    repo,
                }
            })
            .collect();

    let forges = remotes.iter().filter_map(|r| r.forge).collect();

    Ok(Workspace {
        cwd,
        repo: Some(WorkspaceRepo {
            root: info.root,
            branch: info.branch,
            head_sha: info.head_sha,
            default_branch: info.default_branch,
            dirty: info.has_changes,
            remotes,
        }),
        forges,
    })
}
