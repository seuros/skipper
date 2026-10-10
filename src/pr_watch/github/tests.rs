use super::*;

fn pr(state: &str, merged: bool, sha: &str) -> RestPr {
    serde_json::from_value(serde_json::json!({
        "state": state, "merged": merged, "merged_at": null, "merged_by": {"login": "bob"},
        "merge_commit_sha": null, "closed_at": null, "updated_at": "t", "html_url": "u",
        "comments": 0, "review_comments": 0, "head": {"sha": sha}
    }))
    .unwrap()
}

#[test]
fn test_transitions() {
    let open = pr("open", false, "a");
    let kinds = |events: Vec<Event>| events.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>();

    assert!(transitions(&open, &open).is_empty());
    let pushed_and_merged = kinds(transitions(&open, &pr("closed", true, "b")));
    assert!(pushed_and_merged[0].starts_with("Push"));
    assert!(pushed_and_merged[1].starts_with("Merged { by: Some(\"bob\")"));
    assert!(kinds(transitions(&open, &pr("closed", false, "a")))[0].starts_with("Closed"));
}

#[test]
fn test_summarize_checks() {
    let run = |status: &str, conclusion: Option<&str>| CheckRun {
        name: "ci".into(),
        status: status.into(),
        conclusion: conclusion.map(Into::into),
        html_url: None,
        output: None,
    };
    let status = |state: &str| CommitStatus {
        context: "lint".into(),
        state: state.into(),
        target_url: None,
        description: None,
    };

    let done = summarize("a".into(), &[run("completed", Some("success"))], &[status("success")]);
    assert_eq!((done.conclusion, done.counts.pass), ("success", 2));

    let running = summarize("a".into(), &[run("in_progress", None)], &[]);
    assert_eq!(running.conclusion, "pending");

    // A failure settles the conclusion while other checks still run.
    let failing =
        summarize("a".into(), &[run("completed", Some("timed_out")), run("queued", None)], &[]);
    assert_eq!((failing.conclusion, failing.failed.len()), ("failure", 1));
}
