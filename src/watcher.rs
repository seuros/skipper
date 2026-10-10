use crate::error::{CliError, Result};
use crate::provider::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::{Display, Write as _};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

#[derive(Debug)]
pub struct Notification {
    pub watcher: &'static str,
    pub message: String,
    pub data: Value,
}

impl Notification {
    fn of_state(watcher: &'static str, message: String, state: &WatcherState) -> Self {
        let data = serde_json::to_value(state).unwrap_or_else(|e| {
            tracing::warn!(watcher, error = %e, "watcher state did not serialize");
            Value::Null
        });
        Self { watcher, message, data }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatcherState {
    GitDirty {
        staged: usize,
        modified: usize,
        untracked: usize,
    },
    Forges {
        has_repo: bool,
        forges: BTreeSet<&'static str>,
        /// The remote forge reads go to, as `name (forge)`.
        #[serde(skip_serializing_if = "Option::is_none")]
        current: Option<String>,
    },
}

/// `prefix` then `label: old → new` for every field that moved; `None` when
/// none did.
fn changes<T: PartialEq + Display>(prefix: &str, fields: &[(&str, T, T)]) -> Option<String> {
    let mut out = String::from(prefix);
    for (label, old, new) in fields.iter().filter(|(_, old, new)| old != new) {
        if out.len() > prefix.len() {
            out.push_str(", ");
        }
        write!(out, "{label}: {old} → {new}").expect("writing to a String cannot fail");
    }
    (out.len() > prefix.len()).then_some(out)
}

pub type WatcherResult = Result<WatcherState>;

pub trait Watcher: Send + Sync {
    fn name(&self) -> &'static str;

    fn interval(&self) -> Duration;

    fn check(&self) -> BoxFuture<'_, WatcherResult>;

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification>;
}

pub struct WatcherManager {
    tx: mpsc::UnboundedSender<Notification>,
    debounce: Duration,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl WatcherManager {
    pub fn new(tx: mpsc::UnboundedSender<Notification>) -> Self {
        Self { tx, debounce: Duration::from_secs(2), tasks: Mutex::default() }
    }

    /// Polls `watcher` on its interval. The first state is the baseline; a
    /// later one that differs is reported unless the last report is younger
    /// than the debounce, in which case it is compared again next tick.
    pub fn add(&self, watcher: Arc<dyn Watcher>) {
        let tx = self.tx.clone();
        let debounce = self.debounce;

        let handle = tokio::spawn(async move {
            let name = watcher.name();
            let mut ticker = tokio::time::interval(watcher.interval());
            let mut reported: Option<(WatcherState, Instant)> = None;

            loop {
                ticker.tick().await;

                let state = match watcher.check().await {
                    Ok(state) => state,
                    Err(e) => {
                        tracing::warn!(watcher = name, error = ?e, "watcher check failed");
                        continue;
                    }
                };

                let now = Instant::now();
                let old = match &reported {
                    None => &state,
                    Some((old, _)) if *old == state => continue,
                    Some((_, at)) if now.duration_since(*at) < debounce => continue,
                    Some((old, _)) => old,
                };

                if let Some(notification) = watcher.on_change(old, &state)
                    && let Err(e) = tx.send(notification)
                {
                    tracing::error!(watcher = name, error = ?e, "failed to send notification");
                }
                reported = Some((state, now));
            }
        });

        self.tasks.lock().expect("watcher tasks lock").push(handle);
    }

    pub fn stop(&self) {
        for task in self.tasks.lock().expect("watcher tasks lock").drain(..) {
            task.abort();
        }
    }
}

pub struct GitStatusWatcher {
    cwd: std::path::PathBuf,
    interval: Duration,
}

impl GitStatusWatcher {
    pub fn new(cwd: impl Into<std::path::PathBuf>) -> Self {
        Self { cwd: cwd.into(), interval: Duration::from_secs(5) }
    }
}

impl Watcher for GitStatusWatcher {
    fn name(&self) -> &'static str {
        "git_status"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn check(&self) -> BoxFuture<'_, WatcherResult> {
        Box::pin(async move {
            let counts = crate::git::status_counts(&self.cwd)
                .map_err(|e| CliError::execution_failed("git", 1, e.to_string()))?;
            Ok(WatcherState::GitDirty {
                staged: counts.staged,
                modified: counts.unstaged,
                untracked: counts.untracked,
            })
        })
    }

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification> {
        let (
            WatcherState::GitDirty { staged: old_s, modified: old_m, untracked: old_u },
            WatcherState::GitDirty { staged: new_s, modified: new_m, untracked: new_u },
        ) = (old, new)
        else {
            return None;
        };
        let message = changes(
            "Git status changed: ",
            &[("staged", old_s, new_s), ("modified", old_m, new_m), ("untracked", old_u, new_u)],
        )?;

        Some(Notification::of_state(self.name(), message, new))
    }
}

pub struct RemoteWatcher {
    env: Arc<crate::environment::SkipperEnvironment>,
    interval: Duration,
}

impl RemoteWatcher {
    pub const NAME: &'static str = "remotes";

    pub const fn new(env: Arc<crate::environment::SkipperEnvironment>) -> Self {
        Self { env, interval: Duration::from_secs(5) }
    }
}

impl Watcher for RemoteWatcher {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn check(&self) -> BoxFuture<'_, WatcherResult> {
        Box::pin(async move {
            use crate::environment::Environment as _;

            self.env.refresh();
            Ok(WatcherState::Forges {
                has_repo: self.env.has_git_repo(),
                forges: self.env.forges(),
                current: self.env.current_remote_label(),
            })
        })
    }

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification> {
        let (
            WatcherState::Forges { has_repo: was_repo, forges: old_forges, current: old_current },
            WatcherState::Forges { has_repo, forges, current },
        ) = (old, new)
        else {
            return None;
        };

        if was_repo == has_repo && old_forges == forges && old_current == current {
            return None;
        }

        let mut message = if !has_repo {
            "Workspace is no longer a git repository; forge tools hidden".to_string()
        } else if forges.is_empty() {
            "No remote maps to a known forge; forge tools hidden".to_string()
        } else {
            let mut message = String::from("Forges available: ");
            for (i, forge) in forges.iter().enumerate() {
                if i > 0 {
                    message.push_str(", ");
                }
                message.push_str(forge);
            }
            message
        };
        if *has_repo && old_current != current {
            message.push_str("; current remote: ");
            message.push_str(current.as_deref().unwrap_or("none"));
        }

        Some(Notification::of_state(Self::NAME, message, new))
    }
}

#[cfg(test)]
mod tests;
