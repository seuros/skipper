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
    /// `owner/name`, written where it is used instead of into a `String` first.
    pub fn full_name(&self) -> impl std::fmt::Display + '_ {
        std::fmt::from_fn(|f| write!(f, "{}/{}", self.owner, self.name))
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

    #[error("repo={0:?} is neither a remote ({1}) nor owner/name")]
    BadRepo(String, String),

    #[error("remote {remote} is on {actual}, not {forge}")]
    WrongForge { remote: String, actual: &'static str, forge: &'static str },
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

    let CurrentRemote { name, url, .. } = current;
    locate(env, name, &url)
}

/// The forge repo remote `remote` points at with `url`. The host is parsed
/// once and serves the forge lookup too.
fn locate(env: &SkipperEnvironment, remote: String, url: &str) -> Result<ForgeRepo, RemoteError> {
    let located = crate::remote::host_of(url).zip(crate::remote::repo_path_of(url));
    let Some((host, (owner, name))) = located else {
        return Err(RemoteError::NotARepoUrl { remote, url: crate::remote::redact_url(url) });
    };
    let Some(forge) = env.forge_for_host(&host) else {
        return Err(RemoteError::Unmapped { remote, host });
    };
    Ok(ForgeRepo { remote, forge, host, owner, name })
}

/// The repo on `forge`: the current remote when it is there, else the one repo
/// the remotes have on that forge.
///
/// Forge-specific reads (GitHub PR checks) keep working while the branch
/// tracks a mirror on another forge.
pub fn forge_repo_on(
    env: &SkipperEnvironment,
    forge: &'static str,
) -> Result<ForgeRepo, RemoteError> {
    let (info, current) = crate::git::remotes_with_current(env.cwd())?;
    if let Some(current) = current
        && let Ok(repo) = locate(env, current.name, &current.url)
        && repo.forge == forge
    {
        return Ok(repo);
    }

    let candidates = info
        .remotes
        .into_iter()
        .filter_map(|(remote, url)| {
            let host = crate::remote::host_of(&url)?;
            (env.forge_for_host(&host)? == forge).then_some(())?;
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

/// The repo a read asked for with `repo=`: a remote name, or `owner/name` on
/// the host of `fallback`. `None` takes `fallback` as is.
fn select(
    env: &SkipperEnvironment,
    spec: Option<&str>,
    fallback: impl FnOnce() -> Result<ForgeRepo, RemoteError>,
) -> Result<ForgeRepo, RemoteError> {
    let Some(spec) = spec.map(str::trim).filter(|s| !s.is_empty()) else {
        return fallback();
    };
    let remotes = crate::git::remotes(env.cwd())?.remotes;
    if let Some(url) = remotes.get(spec) {
        return locate(env, spec.to_owned(), url);
    }
    let Some((owner, name)) = owner_name(spec) else {
        let names: Vec<&str> = remotes.keys().map(String::as_str).collect();
        return Err(RemoteError::BadRepo(spec.to_string(), names.join(", ")));
    };
    let base = fallback()?;
    Ok(ForgeRepo { remote: String::new(), owner, name, ..base })
}

/// `owner/name` when `spec` is exactly that: one slash, name characters only.
pub(crate) fn owner_name(spec: &str) -> Option<(String, String)> {
    let (owner, name) = spec.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name)).then(|| (owner.to_string(), name.to_string()))
}

/// The repo a read on `forge` goes to: `spec` (a remote name or `owner/name`),
/// else the current remote when it is on `forge`, else the one repo the
/// remotes have there.
pub fn select_on(
    env: &SkipperEnvironment,
    forge: &'static str,
    spec: Option<&str>,
) -> Result<ForgeRepo, RemoteError> {
    let repo = select(env, spec, || forge_repo_on(env, forge))?;
    if repo.forge != forge {
        return Err(RemoteError::WrongForge { remote: repo.remote, actual: repo.forge, forge });
    }
    Ok(repo)
}

/// The repo a forge-agnostic read (issues) goes to: `spec`, else the current
/// remote, on whichever forge it is.
pub fn select_any(env: &SkipperEnvironment, spec: Option<&str>) -> Result<ForgeRepo, RemoteError> {
    select(env, spec, || forge_repo(env))
}

pub fn snapshot(env: &SkipperEnvironment) -> Result<Workspace, GitError> {
    let cwd = env.cwd().to_path_buf();

    let (info, remotes) = match crate::git::repo_info_with_remotes(&cwd) {
        Ok(found) => found,
        Err(GitError::NotARepo(_)) => {
            return Ok(Workspace { cwd, repo: None, forges: BTreeSet::new() });
        }
        Err(e) => return Err(e),
    };

    let current = info.remote.as_ref();
    let remotes: Vec<WorkspaceRemote> =
        crate::remote::ordered_remotes(remotes, current.map(|c| c.name.as_str()))
            .into_iter()
            .map(|(name, url)| {
                let host = crate::remote::host_of(&url);
                let (owner, repo) =
                    host.as_ref().and_then(|_| crate::remote::repo_path_of(&url)).unzip();
                WorkspaceRemote {
                    current: current.filter(|c| c.name == name).map(|c| c.source),
                    forge: host.as_deref().and_then(|host| env.forge_for_host(host)),
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
