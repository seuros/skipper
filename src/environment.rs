use crate::git::{CurrentRemote, GitError};
use crate::remote::ForgeHosts;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub trait Environment: Send + Sync {
    fn has_git_repo(&self) -> bool;

    fn git_is_clean(&self) -> bool;

    fn git_has_staged(&self) -> bool;

    fn cwd(&self) -> &Path;

    fn forges(&self) -> BTreeSet<&'static str>;
}

#[derive(Debug, Clone, Default, PartialEq)]
struct RepoState {
    has_repo: bool,
    forges: BTreeSet<&'static str>,
    unknown_hosts: BTreeSet<String>,
    current: Option<CurrentRemote>,
}

pub struct SkipperEnvironment {
    cwd: PathBuf,
    hosts: ForgeHosts,
    state: RwLock<RepoState>,
}

impl SkipperEnvironment {
    pub fn with_hosts(cwd: impl Into<PathBuf>, hosts: ForgeHosts) -> Self {
        let env = Self { cwd: cwd.into(), hosts, state: RwLock::new(RepoState::default()) };
        env.refresh();
        env
    }

    pub fn refresh(&self) -> bool {
        let next = self.detect();

        let mut state = self.state.write().expect("environment state lock poisoned");
        if *state == next {
            return false;
        }

        if !next.unknown_hosts.is_empty() && state.unknown_hosts != next.unknown_hosts {
            tracing::info!(
                hosts = ?next.unknown_hosts,
                "remote hosts not mapped to a forge; add them under [hosts] in skipper's config"
            );
        }

        *state = next;
        true
    }

    /// One open of the repository: its remotes, their forges, and the
    /// current remote. Runs before every tool listing and call.
    fn detect(&self) -> RepoState {
        let (info, current) = match crate::git::remotes_with_current(&self.cwd) {
            Ok(found) => found,
            Err(GitError::NotARepo(_)) => return RepoState::default(),
            Err(e) => {
                tracing::warn!(error = %e, "could not read git remotes; forge tools stay hidden");
                return RepoState { has_repo: true, ..RepoState::default() };
            }
        };
        let (forges, unknown_hosts) = self.hosts.classify(info.remotes.values());
        RepoState { has_repo: true, forges, unknown_hosts, current }
    }

    fn state(&self) -> std::sync::RwLockReadGuard<'_, RepoState> {
        self.state.read().expect("environment state lock poisoned")
    }

    pub fn unknown_hosts(&self) -> BTreeSet<String> {
        self.state().unknown_hosts.clone()
    }

    /// Whether a remote of the workspace is on `forge`, without copying the set.
    pub fn has_forge(&self, forge: &str) -> bool {
        self.state().forges.contains(forge)
    }

    pub fn forge_for_url(&self, url: &str) -> Option<&'static str> {
        self.hosts.provider_for_url(url)
    }

    /// The forge mapped to `host`, a host [`crate::remote::host_of`] gave.
    pub fn forge_for_host(&self, host: &str) -> Option<&'static str> {
        self.hosts.provider_for_host(host)
    }

    /// The current remote as of the last [`Self::refresh`], as `name (forge)`.
    pub fn current_remote_label(&self) -> Option<String> {
        let state = self.state();
        let current = state.current.as_ref()?;
        let mut label = current.name.clone();
        if let Some(forge) = self.forge_for_url(&current.url) {
            label.push_str(" (");
            label.push_str(forge);
            label.push(')');
        }
        Some(label)
    }
}

impl Environment for SkipperEnvironment {
    fn has_git_repo(&self) -> bool {
        self.state().has_repo
    }

    fn git_is_clean(&self) -> bool {
        if !self.has_git_repo() {
            return true;
        }

        crate::git::is_clean(&self.cwd).unwrap_or(true)
    }

    fn git_has_staged(&self) -> bool {
        if !self.has_git_repo() {
            return false;
        }

        crate::git::has_staged(&self.cwd).unwrap_or(false)
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }

    fn forges(&self) -> BTreeSet<&'static str> {
        self.state().forges.clone()
    }
}

#[cfg(test)]
mod tests;
