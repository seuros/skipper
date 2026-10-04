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

    #[error("no remote on {0}")]
    NoForgeRemote(&'static str),

    #[error(
        "remotes {remotes} are different {forge} repos and the current remote is none of them; \
         set the branch upstream to one"
    )]
    AmbiguousForge { forge: &'static str, remotes: String },
}

impl From<RemoteError> for crate::error::CliError {
    fn from(e: RemoteError) -> Self {
        Self::no_target(e.to_string())
    }
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

/// The repo on `forge`: the current remote when it is there, else the one repo
/// the remotes have on that forge. Forge-specific reads (GitHub PR checks) keep
/// working while the branch tracks a mirror on another forge.
pub fn forge_repo_on(
    env: &SkipperEnvironment,
    forge: &'static str,
) -> Result<ForgeRepo, RemoteError> {
    match forge_repo(env) {
        Ok(current) if current.forge == forge => return Ok(current),
        Err(RemoteError::Git(e)) => return Err(RemoteError::Git(e)),
        _ => {}
    }

    let candidates = crate::git::remotes(env.cwd())?
        .remotes
        .into_iter()
        .filter_map(|(remote, url)| {
            env.forge_for_url(&url).filter(|f| *f == forge)?;
            let host = crate::remote::host_of(&url)?;
            let (owner, name) = crate::remote::repo_path_of(&url)?;
            Some(ForgeRepo { remote, forge, host, owner, name })
        })
        .collect();
    pick_on(forge, candidates)
}

/// The single repo among `candidates`; remotes naming the same repo count once.
fn pick_on(forge: &'static str, mut candidates: Vec<ForgeRepo>) -> Result<ForgeRepo, RemoteError> {
    candidates.sort_by(|a, b| {
        (&a.host, &a.owner, &a.name, &a.remote).cmp(&(&b.host, &b.owner, &b.name, &b.remote))
    });
    candidates.dedup_by(|b, a| (&a.host, &a.owner, &a.name) == (&b.host, &b.owner, &b.name));
    match candidates.len() {
        0 => Err(RemoteError::NoForgeRemote(forge)),
        1 => Ok(candidates.remove(0)),
        _ => Err(RemoteError::AmbiguousForge {
            forge,
            remotes: candidates.iter().map(|c| c.remote.as_str()).collect::<Vec<_>>().join(", "),
        }),
    }
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

#[cfg(test)]
mod tests;
