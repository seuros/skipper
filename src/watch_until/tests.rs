use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const T0: &str = "2026-11-19T09:00:00Z";

fn at(secs: u64) -> String {
    (T0.parse::<Timestamp>().unwrap() + Duration::from_secs(secs)).to_string()
}

/// A repo whose release PR opens, merges and builds at set times after the
/// call, beside an older finished run of the same workflow.
struct Release {
    begun: Instant,
    opens: u64,
    merges: Option<u64>,
    closes: Option<u64>,
    run_starts: u64,
    run_ends: u64,
    run_status: &'static str,
    polls: AtomicUsize,
}

impl Release {
    fn new() -> Self {
        Self {
            begun: Instant::now(),
            opens: 40,
            merges: Some(100),
            closes: None,
            run_starts: 110,
            run_ends: 400,
            run_status: "success",
            polls: AtomicUsize::new(0),
        }
    }

    fn elapsed(&self) -> u64 {
        self.polls.fetch_add(1, Ordering::Relaxed);
        self.begun.elapsed().as_secs()
    }

    fn pr(&self, now: u64) -> PrState {
        let merged = self.merges.filter(|at| now >= *at);
        let closed = self.closes.is_some_and(|at| now >= at);
        PrState {
            pr: 7,
            title: "chore(master): release 1.2.3".to_string(),
            head: "release-please--branches--master".to_string(),
            state: if merged.is_some() {
                "merged"
            } else if closed {
                "closed"
            } else {
                "open"
            }
            .to_string(),
            merged_at: merged.map(at),
        }
    }
}

fn build(id: &str, status: &'static str, branch: &str, created: String) -> DatedRun {
    DatedRun {
        run: BuildRun {
            id: id.to_string(),
            status,
            branch: Some(branch.to_string()),
            workflow: Some("Release - Build & Push".to_string()),
            title: None,
            url: None,
        },
        created_at: Some(created),
        head_sha: Some(
            format!("{id}{id}{id}{id}{id}{id}{id}0000000000000000000000000000000000")[..40]
                .to_string(),
        ),
    }
}

impl Release {
    fn open_now(&self) -> Vec<OpenPr> {
        let now = self.elapsed();
        if now < self.opens || self.pr(now).state != "open" {
            return Vec::new();
        }
        vec![OpenPr {
            pr: self.pr(now),
            body: "* fix things ([abc1234](https://example.test))".to_string(),
            created_at: at(self.opens),
        }]
    }

    fn runs_now(&self) -> Vec<DatedRun> {
        let now = self.elapsed();
        let mut runs = Vec::new();
        if let Some(next) = self.run_starts.checked_add(60).filter(|next| now >= *next) {
            runs.push(build("3", "success", "v1.3.0", at(next)));
        }
        if now >= self.run_starts {
            let status = if now >= self.run_ends { self.run_status } else { "queued" };
            runs.push(build("2", status, "v1.2.3", at(self.run_starts)));
        }
        runs.push(build("1", "success", "v1.2.2", "2026-11-18T09:00:00Z".to_string()));
        runs
    }
}

impl Forge for Release {
    fn open_prs(&self) -> impl Future<Output = Result<Vec<OpenPr>>> + Send {
        std::future::ready(Ok(self.open_now()))
    }

    fn pr_state(&self, _number: u64) -> impl Future<Output = Result<PrState>> + Send {
        std::future::ready(Ok(self.pr(self.elapsed())))
    }

    fn runs(&self, _branch: Option<&str>) -> impl Future<Output = Result<Vec<DatedRun>>> + Send {
        std::future::ready(Ok(self.runs_now()))
    }
}

fn release_chain() -> Vec<Step> {
    serde_json::from_value(serde_json::json!([
        { "wait_for": "pr_opened", "title_contains": "RELEASE", "mentions": "abc1234" },
        { "wait_for": "pr_merged" },
        { "wait_for": "run_finished", "workflow": "release - build" }
    ]))
    .expect("steps deserialize")
}

fn started() -> Timestamp {
    T0.parse().unwrap()
}

#[tokio::test(start_paused = true)]
async fn test_chain_follows_a_release_from_pr_to_build() {
    let forge = Release::new();
    let outcome = run(&forge, &release_chain(), Duration::from_hours(1), started()).await;

    assert!(outcome.completed && !outcome.failed && !outcome.expired, "{outcome:?}");
    let states: Vec<_> = outcome.steps.iter().map(|s| (s.wait_for, s.state)).collect();
    assert_eq!(states, [("pr_opened", "done"), ("pr_merged", "done"), ("run_finished", "done")]);
    assert_eq!(outcome.steps[0].pr.as_ref().unwrap().pr, 7);
    assert_eq!(outcome.steps[1].at.as_deref(), Some(at(100).as_str()));
    // Not the v1.2.2 run from before the merge, nor the v1.3.0 run of a
    // release that landed while this one was still building.
    assert_eq!(outcome.steps[2].run.as_ref().unwrap().id, "2");
    assert!((400..=430).contains(&outcome.waited_secs), "{}", outcome.waited_secs);
}

#[tokio::test(start_paused = true)]
async fn test_chain_expires_naming_the_step_still_waiting() {
    let forge = Release { merges: None, ..Release::new() };
    let outcome = run(&forge, &release_chain(), Duration::from_mins(10), started()).await;

    assert!(outcome.expired && !outcome.completed && !outcome.failed);
    let states: Vec<_> = outcome.steps.iter().map(|s| s.state).collect();
    assert_eq!(states, ["done", "waiting", "not_reached"]);
    assert_eq!(outcome.steps[1].pr.as_ref().unwrap().state, "open");
    assert_eq!(outcome.waited_secs, 600);
    // 15s polls for a step's first five minutes, 30s after.
    assert!(forge.polls.load(Ordering::Relaxed) < 45);
}

#[tokio::test(start_paused = true)]
async fn test_chain_fails_on_a_pr_closed_unmerged_or_a_failed_run() {
    let closed = Release { merges: None, closes: Some(90), ..Release::new() };
    let outcome = run(&closed, &release_chain(), Duration::from_hours(1), started()).await;
    assert!(outcome.failed && !outcome.expired && !outcome.completed);
    let states: Vec<_> = outcome.steps.iter().map(|s| s.state).collect();
    assert_eq!(states, ["done", "failed", "not_reached"]);

    let broken = Release { run_status: "failure", ..Release::new() };
    let outcome = run(&broken, &release_chain(), Duration::from_hours(1), started()).await;
    assert!(outcome.failed);
    assert_eq!(outcome.steps[2].state, "failed");
    assert_eq!(outcome.steps[2].run.as_ref().unwrap().status, "failure");
}

#[tokio::test(start_paused = true)]
async fn test_first_run_step_skips_runs_finished_before_the_call() {
    let forge = Release::new();
    let unpinned: Vec<Step> = serde_json::from_value(serde_json::json!([
        { "wait_for": "run_finished", "workflow": "Release" }
    ]))
    .unwrap();
    let outcome = run(&forge, &unpinned, Duration::from_hours(1), started()).await;
    assert_eq!(outcome.steps[0].run.as_ref().unwrap().id, "2");

    let pinned: Vec<Step> = serde_json::from_value(serde_json::json!([
        { "wait_for": "run_finished", "branch": "v1.2.2" }
    ]))
    .unwrap();
    let forge = Release { run_starts: u64::MAX, ..Release::new() };
    let outcome = run(&forge, &pinned, Duration::from_hours(1), started()).await;
    assert!(outcome.completed);
    assert_eq!(outcome.steps[0].run.as_ref().unwrap().id, "1");
    assert_eq!(outcome.waited_secs, 0);
}

#[tokio::test(start_paused = true)]
async fn test_commit_waits_for_its_own_run_on_a_reused_tag() {
    // Run 1 is an earlier, finished run; with the tag alone it would answer.
    let forge = Release::new();
    let steps: Vec<Step> = serde_json::from_value(serde_json::json!([
        { "wait_for": "run_finished", "workflow": "release", "commit": "2222222" }
    ]))
    .unwrap();
    let outcome = run(&forge, &steps, Duration::from_hours(1), started()).await;

    assert!(outcome.completed, "{outcome:?}");
    assert_eq!(outcome.steps[0].run.as_ref().unwrap().id, "2");
    assert!(outcome.waited_secs >= 400);

    let after_merge: Vec<Step> = serde_json::from_value(serde_json::json!([
        { "wait_for": "pr_merged", "pr": 7 },
        { "wait_for": "run_finished", "commit": "3333333" }
    ]))
    .unwrap();
    let outcome = run(&Release::new(), &after_merge, Duration::from_hours(1), started()).await;
    assert_eq!(outcome.steps[1].run.as_ref().unwrap().id, "3");
}

#[test]
fn test_validate_names_the_step_to_fix() {
    let steps = |value| serde_json::from_value::<Vec<Step>>(value).unwrap();

    assert!(validate(&release_chain()).is_ok());
    assert!(validate(&[]).unwrap_err().contains("1 to 6"));
    let error = validate(&steps(serde_json::json!([{ "wait_for": "pr_opened" }]))).unwrap_err();
    assert!(error.contains("step 0 (pr_opened)"), "{error}");
    let error = validate(&steps(serde_json::json!([{ "wait_for": "pr_merged" }]))).unwrap_err();
    assert!(error.contains("needs `pr`"), "{error}");
    let error = validate(&steps(serde_json::json!([
        { "wait_for": "pr_merged", "pr": 3 },
        { "wait_for": "run_finished", "workflow": " " }
    ])))
    .unwrap_err();
    assert!(error.contains("step 1 (run_finished)"), "{error}");
    let error = validate(&steps(serde_json::json!([
        { "wait_for": "run_finished", "commit": "abc" }
    ])))
    .unwrap_err();
    assert!(error.contains("7 to 40 hex"), "{error}");
}
