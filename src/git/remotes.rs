use std::collections::BTreeMap;
use std::path::Path;

use gix::bstr::{ByteSlice, ByteVec};
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
    /// The checked-out branch tracks it (`branch.<name>.remote`).
    Tracked,
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

/// Every remote and the current one, from one read of the config.
pub fn with_current(cwd: &Path) -> Result<(RemoteInfo, Option<CurrentRemote>), GitError> {
    let repo = open_repo(cwd)?;
    let (remotes, current) = with_current_in(&repo);
    Ok((RemoteInfo { remotes }, current))
}

pub(crate) fn with_current_in(
    repo: &gix::Repository,
) -> (BTreeMap<String, String>, Option<CurrentRemote>) {
    let remotes = remote_urls(repo);
    let upstream = upstream(repo);
    let current = pick_in(upstream.as_ref().and_then(gix::remote::Name::as_symbol), &remotes);
    (remotes, current)
}

/// Read from config on every call: checking out a branch that tracks another
/// remote, or `git branch -u`, switches it at once.
pub(crate) fn current_in(repo: &gix::Repository) -> Option<CurrentRemote> {
    let upstream = upstream(repo);
    pick(upstream.as_ref().and_then(gix::remote::Name::as_symbol), remote_urls(repo))
}

/// The remote the checked-out branch tracks, as configured. A URL upstream
/// names no remote.
fn upstream(repo: &gix::Repository) -> Option<gix::remote::Name<'_>> {
    let head = repo.head_name().ok().flatten()?;
    repo.branch_remote_name(head.shorten(), gix::remote::Direction::Fetch)
}

/// The upstream when it names a configured remote, else the only remote,
/// else `origin`. `None` when several remotes leave it open. An upstream of
/// `.` (a local branch) or a bare URL names no remote and falls through.
fn choose(upstream: Option<&str>, remotes: &BTreeMap<String, String>) -> Option<RemoteSource> {
    if upstream.is_some_and(|name| remotes.contains_key(name)) {
        Some(RemoteSource::Tracked)
    } else if remotes.len() == 1 {
        Some(RemoteSource::Only)
    } else if remotes.contains_key("origin") {
        Some(RemoteSource::Origin)
    } else {
        None
    }
}

/// The current remote, see [`choose`], moved out of `remotes`.
pub(crate) fn pick(
    upstream: Option<&str>,
    mut remotes: BTreeMap<String, String>,
) -> Option<CurrentRemote> {
    let source = choose(upstream, &remotes)?;
    let (name, url) = match source {
        RemoteSource::Tracked => remotes.remove_entry(upstream?),
        RemoteSource::Only => remotes.pop_first(),
        RemoteSource::Origin => remotes.remove_entry("origin"),
    }?;
    Some(CurrentRemote { name, url, source })
}

/// [`pick`] from remotes the caller keeps: only the chosen entry is copied.
fn pick_in(upstream: Option<&str>, remotes: &BTreeMap<String, String>) -> Option<CurrentRemote> {
    let source = choose(upstream, remotes)?;
    let (name, url) = match source {
        RemoteSource::Tracked => remotes.get_key_value(upstream?),
        RemoteSource::Only => remotes.first_key_value(),
        RemoteSource::Origin => remotes.get_key_value("origin"),
    }?;
    Some(CurrentRemote { name: name.clone(), url: url.clone(), source })
}

fn remote_urls(repo: &gix::Repository) -> BTreeMap<String, String> {
    repo.remote_names()
        .iter()
        .filter_map(|name| {
            let remote = repo.find_remote(name.as_bstr()).ok()?;
            let url = remote.url(gix::remote::Direction::Fetch)?;
            // `to_bstring` already allocates; take its buffer for the String.
            let url = Vec::from(url.to_bstring()).into_string_lossy();
            Some((name.to_str_lossy().into_owned(), url))
        })
        .collect()
}

#[cfg(test)]
mod tests;
