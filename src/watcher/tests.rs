use super::*;
use std::sync::Mutex;

/// MOCKED: replays scripted staged counts instead of reading git; the
/// message comes from the real `GitStatusWatcher`.
struct Scripted(Mutex<std::vec::IntoIter<usize>>);

impl Watcher for Scripted {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn check(&self) -> BoxFuture<'_, WatcherResult> {
        let next = self.0.lock().expect("script lock").next();
        Box::pin(async move {
            let staged = next.ok_or_else(|| CliError::no_target("script exhausted"))?;
            Ok(WatcherState::GitDirty { staged, modified: 0, untracked: 0 })
        })
    }

    fn on_change(&self, old: &WatcherState, new: &WatcherState) -> Option<Notification> {
        GitStatusWatcher::new(".").on_change(old, new)
    }
}

#[tokio::test(start_paused = true)]
async fn test_manager_reports_changes_after_debounce() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let manager = WatcherManager { tx, debounce: Duration::from_secs(2), tasks: Mutex::default() };
    let start = Instant::now();

    // t0 baseline, t1 change inside the debounce, t2 the same change once
    // it has passed, t3 unchanged, t4 a new change.
    manager.add(Arc::new(Scripted(Mutex::new(vec![0, 1, 1, 1, 2].into_iter()))));

    let reports =
        [(2, "Git status changed: staged: 0 → 1", 1), (4, "Git status changed: staged: 1 → 2", 2)];
    for (secs, message, staged) in reports {
        let note = rx.recv().await.expect("watcher reports the change");
        assert_eq!(
            (start.elapsed().as_secs(), note.message.as_str(), &note.data["staged"]),
            (secs, message, &serde_json::json!(staged))
        );
    }
    manager.stop();
}

#[tokio::test]
async fn test_remote_watcher_reports_current_remote_switch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let env = Arc::new(crate::environment::SkipperEnvironment::new(temp.path()));
    let watcher = RemoteWatcher::new(env);
    let state = |current: &str| WatcherState::Forges {
        has_repo: true,
        forges: BTreeSet::from(["github", "tea"]),
        current: Some(current.to_string()),
    };

    let note = watcher
        .on_change(&state("origin (tea)"), &state("github (github)"))
        .expect("a switched current remote is reported");
    assert_eq!(note.message, "Forges available: github, tea; current remote: github (github)");
    assert!(watcher.on_change(&state("github (github)"), &state("github (github)")).is_none());
}
