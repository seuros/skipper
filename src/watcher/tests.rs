use super::*;

#[test]
fn test_watcher_state_is_clean() {
    let clean = WatcherState::GitDirty { staged: 0, modified: 0, untracked: 0 };
    assert!(clean.is_clean());

    let dirty = WatcherState::GitDirty { staged: 1, modified: 0, untracked: 0 };
    assert!(!dirty.is_clean());
}

#[test]
fn test_watcher_state_equality() {
    let s1 = WatcherState::GitDirty { staged: 1, modified: 2, untracked: 3 };
    let s2 = WatcherState::GitDirty { staged: 1, modified: 2, untracked: 3 };
    let s3 = WatcherState::GitDirty { staged: 1, modified: 2, untracked: 4 };

    assert_eq!(s1, s2);
    assert_ne!(s1, s3);
}

struct StaticBuildWatcher;

impl Watcher for StaticBuildWatcher {
    fn name(&self) -> &str {
        "build_test_1"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }

    fn check(&self) -> BoxFuture<'_, WatcherResult> {
        Box::pin(async { Ok(WatcherState::GitDirty { staged: 0, modified: 0, untracked: 0 }) })
    }

    fn on_change(&self, _old: &WatcherState, _new: &WatcherState) -> Option<Notification> {
        None
    }
}

#[tokio::test]
async fn test_manager_remove_stops_watcher() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let manager = WatcherManager::new(tx);

    manager.add(Arc::new(StaticBuildWatcher)).await;
    assert!(manager.watching("build_test_1"));
    assert_eq!(manager.active_names(), vec!["build_test_1".to_string()]);

    assert!(manager.remove("build_test_1").await);
    assert!(!manager.watching("build_test_1"));
    assert!(manager.get_state("build_test_1").await.is_none());

    assert!(!manager.remove("build_test_1").await);
}
