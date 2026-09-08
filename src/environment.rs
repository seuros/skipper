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
}

pub struct SkipperEnvironment {
    cwd: PathBuf,
    hosts: ForgeHosts,
    state: RwLock<RepoState>,
}

impl SkipperEnvironment {
    pub async fn new(cwd: impl Into<PathBuf>) -> Self {
        Self::with_hosts(cwd, ForgeHosts::with_defaults()).await
    }

    pub async fn with_hosts(cwd: impl Into<PathBuf>, hosts: ForgeHosts) -> Self {
        let env = Self { cwd: cwd.into(), hosts, state: RwLock::new(RepoState::default()) };
        env.refresh().await;
        env
    }

    pub async fn refresh(&self) -> bool {
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

    fn detect(&self) -> RepoState {
        if crate::git::repo_root(&self.cwd).is_err() {
            return RepoState::default();
        }

        let urls: Vec<String> = match crate::git::remotes(&self.cwd) {
            Ok(info) => info.remotes.into_values().collect(),
            Err(e) => {
                tracing::warn!(error = %e, "could not read git remotes; forge tools stay hidden");
                Vec::new()
            }
        };

        RepoState {
            has_repo: true,
            forges: self.hosts.providers_for_urls(&urls),
            unknown_hosts: self.hosts.unknown_hosts(&urls),
        }
    }

    fn state(&self) -> RepoState {
        self.state.read().expect("environment state lock poisoned").clone()
    }

    pub fn unknown_hosts(&self) -> BTreeSet<String> {
        self.state().unknown_hosts
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

        crate::git::status(&self.cwd)
            .map(|s| s.staged.is_empty() && s.unstaged.is_empty() && s.untracked.is_empty())
            .unwrap_or(true)
    }

    fn git_has_staged(&self) -> bool {
        if !self.has_git_repo() {
            return false;
        }

        crate::git::status(&self.cwd).map(|s| !s.staged.is_empty()).unwrap_or(false)
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }

    fn forges(&self) -> BTreeSet<&'static str> {
        self.state().forges
    }
}

#[cfg(test)]
mod tests;
