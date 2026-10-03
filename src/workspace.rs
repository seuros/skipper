use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;

use crate::environment::{Environment as _, SkipperEnvironment};
use crate::git::{CurrentRemote, GitError, RemoteSource};

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
    /// The current remote first, then `origin`, the rest by name.
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
    /// Set on the remote forge reads go to, saying why it was picked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<RemoteSource>,
}

/// The forge repository behind the current remote: where issue reads go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeRepo {
    pub remote: String,
    pub forge: &'static str,
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl ForgeRepo {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(transparent)]
    Git(#[from] GitError),

    #[error("repository has no remotes")]
    NoRemotes,

    #[error(
        "no current remote: branch has no upstream and none of {0} is origin; \
         set one with `git branch -u <remote>/<branch>`"
    )]
    Ambiguous(String),

    #[error("remote {remote} names no forge repository: {url}")]
    NotARepoUrl { remote: String, url: String },

    #[error(
        "remote {remote} host {host} is not mapped to a forge; add it under [hosts] in skipper's config"
    )]
    Unmapped { remote: String, host: String },
}

/// Resolved from git config on every call, so a switched upstream applies to
/// the next read.
pub fn forge_repo(env: &SkipperEnvironment) -> Result<ForgeRepo, RemoteError> {
    let Some(current) = crate::git::current_remote(env.cwd())? else {
        let names: Vec<String> = crate::git::remotes(env.cwd())?.remotes.into_keys().collect();
        return Err(if names.is_empty() {
            RemoteError::NoRemotes
        } else {
            RemoteError::Ambiguous(names.join(", "))
        });
    };

    let CurrentRemote { name: remote, url, .. } = current;
    let located = crate::remote::host_of(&url).zip(crate::remote::repo_path_of(&url));
    let Some((host, (owner, name))) = located else {
        return Err(RemoteError::NotARepoUrl { remote, url: crate::remote::redact_url(&url) });
    };
    let Some(forge) = env.forge_for_url(&url) else {
        return Err(RemoteError::Unmapped { remote, host });
    };
    Ok(ForgeRepo { remote, forge, host, owner, name })
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

    let current = info.remote.as_ref();
    let remotes: Vec<WorkspaceRemote> = crate::remote::ordered_remotes(
        crate::git::remotes(&cwd)?.remotes,
        current.map(|c| c.name.as_str()),
    )
    .into_iter()
    .map(|(name, url)| {
        let host = crate::remote::host_of(&url);
        let (owner, repo) = host.as_ref().and_then(|_| crate::remote::repo_path_of(&url)).unzip();
        WorkspaceRemote {
            current: current.filter(|c| c.name == name).map(|c| c.source),
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
