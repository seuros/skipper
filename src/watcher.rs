use crate::error::{CliError, Result};
use crate::provider::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub watcher: String,
    pub message: String,
    pub data: Value,
    pub timestamp: String,
}

impl Notification {
    pub fn new(watcher: impl Into<String>, message: impl Into<String>, data: Value) -> Self {
        Self {
            watcher: watcher.into(),
            message: message.into(),
            data,
            timestamp: jiff::Timestamp::now().to_string(),
        }
    }

    fn of_state(watcher: &str, message: impl Into<String>, state: &WatcherState) -> Self {
        Self::new(watcher, message, serde_json::to_value(state).unwrap_or(Value::Null))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatcherState {
    GitDirty { staged: u32, modified: u32, untracked: u32 },
    Forges { has_repo: bool, forges: Vec<String> },
    Custom(Value),
}

impl WatcherState {
    pub fn is_clean(&self) -> bool {
        match self {
            Self::GitDirty { staged, modified, untracked } => {
                *staged == 0 && *modified == 0 && *untracked == 0
            }
            _ => false,
        }
    }
}

fn changes<T: PartialEq + std::fmt::Display>(fields: &[(&str, T, T)]) -> Option<String> {
    let parts: Vec<String> = fields
        .iter()
        .filter(|(_, old, new)| old != new)
        .map(|(label, old, new)| format!("{label}: {old} → {new}"))
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

pub type WatcherResult = Result<WatcherState>;

pub trait Watcher: Send + Sync {
    fn name(&self) -> &str;

    fn interval(&self) -> Duration;

    fn check(&self) -> BoxFuture<'_, WatcherResult>;

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification>;

    fn done(&self, _state: &WatcherState) -> bool {
        false
    }
}

pub struct WatcherManager {
    tx: mpsc::UnboundedSender<Notification>,
    state: Arc<RwLock<HashMap<String, WatcherState>>>,
    last_change: Arc<RwLock<HashMap<String, Instant>>>,
    debounce: Duration,
    tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
    active: Arc<std::sync::RwLock<HashSet<String>>>,
}

impl WatcherManager {
    pub fn new(tx: mpsc::UnboundedSender<Notification>) -> Self {
        Self {
            tx,
            state: Arc::default(),
            last_change: Arc::default(),
            debounce: Duration::from_secs(2),
            tasks: Arc::default(),
            active: Arc::default(),
        }
    }

    pub fn with_debounce(mut self, duration: Duration) -> Self {
        self.debounce = duration;
        self
    }

    pub async fn add(&self, watcher: Arc<dyn Watcher>) {
        let name = watcher.name().to_string();
        let key = name.clone();
        let interval = watcher.interval();

        let tx = self.tx.clone();
        let state = self.state.clone();
        let last_change = self.last_change.clone();
        let debounce = self.debounce;
        let active = self.active.clone();

        active.write().expect("active watcher lock").insert(name.clone());

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                ticker.tick().await;

                let new_state = match watcher.check().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            watcher = name,
                            error = ?e,
                            "watcher check failed"
                        );
                        continue;
                    }
                };

                let old_state = state.read().await.get(&name).cloned();

                let changed = match &old_state {
                    Some(old) => old != &new_state,
                    None => true,
                };

                if changed {
                    let now = Instant::now();
                    let should_notify = {
                        let last = last_change.read().await;
                        match last.get(&name) {
                            Some(t) => now.duration_since(*t) >= debounce,
                            None => true,
                        }
                    };

                    if should_notify {
                        state.write().await.insert(name.clone(), new_state.clone());
                        last_change.write().await.insert(name.clone(), now);

                        if let Some(notification) =
                            watcher.on_change(old_state.as_ref().unwrap_or(&new_state), &new_state)
                            && let Err(e) = tx.send(notification)
                        {
                            tracing::error!(
                                watcher = name,
                                error = ?e,
                                "failed to send notification"
                            );
                        }
                    }
                }

                if watcher.done(&new_state) {
                    state.write().await.insert(name.clone(), new_state);
                    active.write().expect("active watcher lock").remove(&name);
                    tracing::info!(watcher = name, "watcher reached terminal state, stopping");
                    break;
                }
            }
        });

        if let Some(old) = self.tasks.write().await.insert(key, handle) {
            old.abort();
        }
    }

    pub async fn remove(&self, name: &str) -> bool {
        let removed = match self.tasks.write().await.remove(name) {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        };
        let was_active = self.active.write().expect("active watcher lock").remove(name);
        self.state.write().await.remove(name);
        self.last_change.write().await.remove(name);
        removed || was_active
    }

    pub fn active_names(&self) -> Vec<String> {
        self.active.read().expect("active watcher lock").iter().cloned().collect()
    }

    pub fn has_build_watches(&self) -> bool {
        self.active
            .read()
            .expect("active watcher lock")
            .iter()
            .any(|name| name.starts_with("build_"))
    }

    pub async fn get_state(&self, name: &str) -> Option<WatcherState> {
        self.state.read().await.get(name).cloned()
    }

    pub async fn states(&self) -> HashMap<String, WatcherState> {
        self.state.read().await.clone()
    }

    pub fn watching(&self, name: &str) -> bool {
        self.active.read().expect("active watcher lock").contains(name)
    }

    pub async fn stop(&self) {
        let mut tasks = self.tasks.write().await;
        for (_, task) in tasks.drain() {
            task.abort();
        }
        self.active.write().expect("active watcher lock").clear();
    }
}

pub struct GitStatusWatcher {
    cwd: std::path::PathBuf,
    name: String,
    interval: Duration,
}

impl GitStatusWatcher {
    pub fn new(cwd: impl Into<std::path::PathBuf>) -> Self {
        Self { cwd: cwd.into(), name: "git_status".to_string(), interval: Duration::from_secs(5) }
    }
}

impl Watcher for GitStatusWatcher {
    fn name(&self) -> &str {
        &self.name
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn check(&self) -> BoxFuture<'_, WatcherResult> {
        Box::pin(async move {
            let status = crate::git::status(&self.cwd)
                .map_err(|e| CliError::execution_failed("git", 1, e.to_string()))?;
            Ok(WatcherState::GitDirty {
                staged: status.staged.len() as u32,
                modified: status.unstaged.len() as u32,
                untracked: status.untracked.len() as u32,
            })
        })
    }

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification> {
        let message = match (old, new) {
            (
                WatcherState::GitDirty { staged: old_s, modified: old_m, untracked: old_u },
                WatcherState::GitDirty { staged: new_s, modified: new_m, untracked: new_u },
            ) => {
                let parts = changes(&[
                    ("staged", old_s, new_s),
                    ("modified", old_m, new_m),
                    ("untracked", old_u, new_u),
                ])?;
                format!("Git status changed: {parts}")
            }
            _ => return None,
        };

        Some(Notification::of_state(&self.name, message, new))
    }
}

pub struct RemoteWatcher {
    env: Arc<crate::environment::SkipperEnvironment>,
    name: String,
    interval: Duration,
}

impl RemoteWatcher {
    pub fn new(env: Arc<crate::environment::SkipperEnvironment>) -> Self {
        Self { env, name: "remotes".to_string(), interval: Duration::from_secs(5) }
    }
}

impl Watcher for RemoteWatcher {
    fn name(&self) -> &str {
        &self.name
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn check(&self) -> BoxFuture<'_, WatcherResult> {
        Box::pin(async move {
            use crate::environment::Environment as _;

            self.env.refresh().await;
            Ok(WatcherState::Forges {
                has_repo: self.env.has_git_repo(),
                forges: self.env.forges().into_iter().map(str::to_string).collect(),
            })
        })
    }

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification> {
        let (
            WatcherState::Forges { has_repo: was_repo, forges: old_forges },
            WatcherState::Forges { has_repo, forges },
        ) = (old, new)
        else {
            return None;
        };

        if was_repo == has_repo && old_forges == forges {
            return None;
        }

        let message = if !has_repo {
            "Workspace is no longer a git repository; forge tools hidden".to_string()
        } else if forges.is_empty() {
            "No remote maps to a known forge; forge tools hidden".to_string()
        } else {
            format!("Forges available: {}", forges.join(", "))
        };

        Some(Notification::of_state(&self.name, message, new))
    }
}

#[cfg(test)]
mod tests;
