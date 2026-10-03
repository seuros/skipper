use std::collections::BTreeMap;
use std::path::Path;

use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::git::error::GitError;
use crate::git::open_repo;

#[derive(Debug, Clone, Serialize)]
pub struct RemoteInfo {
    pub remotes: BTreeMap<String, String>,
}

/// Why a remote is the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteSource {
    /// The checked-out branch tracks it.
    Upstream,
    /// The repository has no other remote.
    Only,
    /// Several remotes and no upstream: git's default.
    Origin,
}

/// The remote this checkout talks to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrentRemote {
    pub name: String,
    /// Fetch URL as configured, credentials included; redact before showing.
    pub url: String,
    pub source: RemoteSource,
}

pub fn collect(cwd: &Path) -> Result<RemoteInfo, GitError> {
    let repo = open_repo(cwd)?;
    Ok(RemoteInfo { remotes: remote_urls(&repo) })
}

pub fn current(cwd: &Path) -> Result<Option<CurrentRemote>, GitError> {
    let repo = open_repo(cwd)?;
    Ok(current_in(&repo))
}

/// Read from config on every call: checking out a branch that tracks another
/// remote, or `git branch -u`, switches it at once.
pub(crate) fn current_in(repo: &gix::Repository) -> Option<CurrentRemote> {
    let upstream = repo
        .head_name()
        .ok()
        .flatten()
        .and_then(|head| repo.branch_remote_name(head.shorten(), gix::remote::Direction::Fetch))
        .map(|name| name.as_bstr().to_string());
    pick(upstream.as_deref(), remote_urls(repo))
}

/// The upstream when it names a configured remote, else the only remote,
/// else `origin`. `None` when several remotes leave it open. An upstream of
/// `.` (a local branch) or a bare URL names no remote and falls through.
pub(crate) fn pick(
    upstream: Option<&str>,
    mut remotes: BTreeMap<String, String>,
) -> Option<CurrentRemote> {
    let (name, source) = match upstream.filter(|name| remotes.contains_key(*name)) {
        Some(name) => (name.to_string(), RemoteSource::Upstream),
        None if remotes.len() == 1 => (remotes.keys().next()?.clone(), RemoteSource::Only),
        None if remotes.contains_key("origin") => ("origin".to_string(), RemoteSource::Origin),
        None => return None,
    };
    let url = remotes.remove(&name)?;
    Some(CurrentRemote { name, url, source })
}

fn remote_urls(repo: &gix::Repository) -> BTreeMap<String, String> {
    repo.remote_names()
        .iter()
        .filter_map(|name| {
            let remote = repo.find_remote(name.as_bstr()).ok()?;
            let url = remote.url(gix::remote::Direction::Fetch)?;
            Some((name.to_string(), url.to_bstring().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests;
