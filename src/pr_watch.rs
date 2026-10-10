//! One PR watcher per server: a set of watched PRs, one poller for all of
//! them, and at most one blocking `pr_watch` call waiting on their events.
//!
//! Polling is adaptive per PR (fast while something moves or checks run,
//! backing off while quiet), requests are conditional on `ETags`, and a
//! process-wide token bucket caps the request rate on top.

pub mod github;

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono_machines::{BackoffStrategy, ExponentialBackoff};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use throttle_machines::{Gate, TokenBucket, TokenBucketParams, TokenBucketState};
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use crate::environment::{Environment as _, SkipperEnvironment};
use crate::error::{CliError, Result};
use crate::provider::github::{CheckCounts, GitHubProvider};
use github::{GithubPr, Polled, PrRef};

/// Stop watching after this long without a `pr_watch` call or resource read.
const IDLE_TTL: Duration = Duration::from_mins(30);
/// Never poll one PR more often than this, however often the model asks.
const MIN_GAP: Duration = Duration::from_secs(5);
/// Recent events kept per PR, and finished PRs kept, for the resource.
const RECENT: usize = 10;
/// Under this many requests left in the rate window, spread the rest out.
const LOW_WATER: u64 = 200;

/// Events a watch can wake on besides `merged`/`closed`, which always do.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Comment,
    Review,
    Checks,
    Push,
}

impl EventKind {
    const ALL: [Self; 4] = [Self::Comment, Self::Review, Self::Checks, Self::Push];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`EventKind`]s, as bits: `Copy`, and no allocation per watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventKinds(u8);

impl EventKinds {
    pub const ALL: Self = Self(0b1111);

    pub const fn contains(self, kind: EventKind) -> bool {
        self.0 & kind.bit() != 0
    }
}

impl FromIterator<EventKind> for EventKinds {
    fn from_iter<I: IntoIterator<Item = EventKind>>(kinds: I) -> Self {
        Self(kinds.into_iter().fold(0, |bits, kind| bits | kind.bit()))
    }
}

impl Serialize for EventKinds {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_seq(EventKind::ALL.into_iter().filter(|kind| self.contains(*kind)))
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Merged {
        by: Option<String>,
        sha: Option<String>,
        at: Option<String>,
    },
    Closed {
        at: Option<String>,
    },
    Push {
        sha: String,
    },
    Comment {
        author: String,
        body: String,
        url: String,
        at: String,
        /// Inline review comments only.
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u64>,
    },
    Review {
        author: String,
        /// APPROVED | `CHANGES_REQUESTED` | COMMENTED | DISMISSED
        state: String,
        body: String,
        url: String,
        at: Option<String>,
    },
    Checks(ChecksSummary),
    /// The PR could not be read (gone, no access); its watch ended.
    Error {
        message: String,
    },
}

impl Event {
    /// Merged, closed, and errors end the PR's watch, so they are always
    /// delivered; the rest only when the PR's `until` asks for them.
    const fn wanted(&self, until: EventKinds) -> bool {
        let kind = match self {
            Self::Merged { .. } | Self::Closed { .. } | Self::Error { .. } => return true,
            Self::Comment { .. } => EventKind::Comment,
            Self::Review { .. } => EventKind::Review,
            Self::Checks(_) => EventKind::Checks,
            Self::Push { .. } => EventKind::Push,
        };
        until.contains(kind)
    }

    const fn ends_watch(&self) -> bool {
        matches!(self, Self::Merged { .. } | Self::Closed { .. } | Self::Error { .. })
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ChecksSummary {
    pub sha: String,
    /// success | failure | cancelled | pending | `no_checks`
    pub conclusion: &'static str,
    pub counts: CheckCounts,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<FailedCheck>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FailedCheck {
    pub name: String,
    pub link: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrSnapshot {
    pub pr: u64,
    pub url: String,
    /// open | closed | merged
    pub state: &'static str,
    pub head_sha: String,
    pub checks: Option<ChecksSummary>,
}

/// An event on PR `pr`. The event is shared with the PR's `recent` list.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrEvent {
    pub pr: u64,
    #[serde(flatten)]
    pub event: Arc<Event>,
}

/// Rate headers from the latest response.
#[derive(Debug, Clone, Default)]
pub struct RateHint {
    pub remaining: Option<u64>,
    /// Epoch seconds.
    pub reset: Option<u64>,
    pub retry_after: Option<u64>,
}

/// Process-wide request budget on top of per-PR backoff: bursts of 10,
/// refilled at 30 a minute.
pub struct ApiBudget {
    bucket: Mutex<TokenBucketState>,
    params: TokenBucketParams,
    clock: std::time::Instant,
}

impl ApiBudget {
    fn new() -> Self {
        let params = TokenBucketParams { capacity: 10.0, refill_rate: 0.5 };
        Self {
            bucket: Mutex::new(TokenBucketState { tokens: params.capacity, last_refill: 0.0 }),
            params,
            clock: std::time::Instant::now(),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let retry_after = {
                let mut bucket = self.bucket.lock().expect("api budget lock poisoned");
                let now = self.clock.elapsed().as_secs_f64();
                let decision = TokenBucket::check(*bucket, now, self.params);
                if decision.allowed {
                    *bucket = decision.state;
                    return;
                }
                decision.retry_after
            };
            tokio::time::sleep(Duration::from_secs_f64(retry_after.max(0.05))).await;
        }
    }
}

/// Everything the resource shows, borrowed from the watch state and
/// serialized while it is locked.
#[derive(Serialize)]
struct WatchView<'a> {
    /// A `pr_watch` call is blocking on these PRs now.
    blocking: bool,
    watching: Vec<WatchedView<'a>>,
    /// Events waiting for the next blocking `pr_watch` call.
    undelivered: &'a VecDeque<PrEvent>,
    /// Recently finished watches.
    #[serde(skip_serializing_if = "VecDeque::is_empty")]
    ended: &'a VecDeque<Arc<PrSnapshot>>,
}

#[derive(Serialize)]
struct WatchedView<'a> {
    pr: u64,
    /// owner/name the PR is on.
    repo: &'a str,
    until: EventKinds,
    /// Null until the first poll lands.
    status: Option<&'a PrSnapshot>,
    /// Latest events, including ones `until` does not wake on.
    recent: &'a VecDeque<Arc<Event>>,
    /// Last poll failure; polling continues with backoff.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    next_poll_secs: u64,
}

/// What a discussion read over every watched PR needs: the state of each
/// finished one, and the repo of each still watched.
pub struct DiscussionTargets {
    pub ended: Vec<(u64, &'static str)>,
    pub watching: Vec<(u64, String)>,
}

struct Watched {
    repo: String,
    /// Taken by the poller while a poll is in flight.
    source: Option<GithubPr>,
    until: EventKinds,
    status: Option<Arc<PrSnapshot>>,
    recent: VecDeque<Arc<Event>>,
    error: Option<String>,
    quiet: u8,
    last_poll: Option<Instant>,
    next_poll: Instant,
}

#[derive(Default)]
struct State {
    prs: BTreeMap<u64, Watched>,
    inbox: VecDeque<PrEvent>,
    ended: VecDeque<Arc<PrSnapshot>>,
    running: bool,
    last_touch: Option<Instant>,
}

pub struct PrWatcher {
    env: Arc<SkipperEnvironment>,
    gh: GitHubProvider,
    state: Mutex<State>,
    /// Wakes the poller: a PR was added, or a blocking call wants fresh data.
    poke: Notify,
    /// Bumped whenever the inbox grows.
    inbox_version: watch::Sender<u64>,
    blocking: AtomicBool,
    budget: ApiBudget,
    /// The checked-out branch's PR per `repo:branch`, asked of GitHub once.
    branch_prs: Mutex<HashMap<String, (crate::workspace::ForgeRepo, u64)>>,
}

impl PrWatcher {
    pub fn new(env: Arc<SkipperEnvironment>) -> Arc<Self> {
        Arc::new(Self {
            env,
            gh: GitHubProvider::new(),
            state: Mutex::default(),
            poke: Notify::new(),
            inbox_version: watch::channel(0).0,
            blocking: AtomicBool::new(false),
            budget: ApiBudget::new(),
            branch_prs: Mutex::default(),
        })
    }

    /// Watch `pr` (default: this branch's), or replace its `until` if already
    /// watched. Returns the PR number and whether it is new to the watch.
    pub async fn add(
        self: &Arc<Self>,
        pr: Option<u64>,
        repo: Option<&str>,
        until: EventKinds,
    ) -> Result<(u64, bool)> {
        let (repo, number) = match pr {
            Some(number) => self.gh.locate_pr(&self.env, repo, Some(number)).await?,
            None => self.branch_pr(repo).await?,
        };

        let (added, spawn) = {
            let mut state = self.lock();
            state.last_touch = Some(Instant::now());
            let added = match state.prs.entry(number) {
                Entry::Occupied(mut watched) => {
                    watched.get_mut().until = until;
                    false
                }
                Entry::Vacant(slot) => {
                    let full_name = repo.full_name().to_string();
                    let pr = PrRef { host: repo.host, owner: repo.owner, repo: repo.name, number };
                    slot.insert(Watched::new(full_name, GithubPr::new(pr), until));
                    true
                }
            };
            let spawn = !state.running;
            state.running = true;
            (added, spawn)
        };

        if spawn {
            tokio::spawn(Arc::clone(self).run());
        }
        self.poke.notify_one();
        Ok((number, added))
    }

    /// Claim the single blocking slot; `None` while another call holds it.
    pub fn try_block(self: &Arc<Self>) -> Option<Blocker> {
        self.blocking.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok()?;

        // A waiting model wants fresh data: pull every PR's next poll in.
        let now = Instant::now();
        for watched in self.lock().prs.values_mut() {
            let earliest = watched.last_poll.map_or(now, |at| (at + MIN_GAP).max(now));
            watched.next_poll = watched.next_poll.min(earliest);
        }
        self.poke.notify_one();
        Some(Blocker { watcher: Arc::clone(self) })
    }

    /// State for the resource, as JSON. Reading counts as interest in the watch.
    pub fn view_json(&self) -> serde_json::Result<String> {
        let mut state = self.lock();
        let now = Instant::now();
        state.last_touch = Some(now);
        let view = WatchView {
            blocking: self.blocking.load(Ordering::SeqCst),
            watching: state
                .prs
                .iter()
                .map(|(pr, w)| WatchedView {
                    pr: *pr,
                    repo: &w.repo,
                    until: w.until,
                    status: w.status.as_deref(),
                    recent: &w.recent,
                    error: w.error.as_deref(),
                    next_poll_secs: w.next_poll.saturating_duration_since(now).as_secs(),
                })
                .collect(),
            undelivered: &state.inbox,
            ended: &state.ended,
        };
        serde_json::to_string(&view)
    }

    /// The PRs a discussion read covers. Reading counts as interest.
    pub fn discussion_targets(&self) -> DiscussionTargets {
        let mut state = self.lock();
        state.last_touch = Some(Instant::now());
        DiscussionTargets {
            ended: state.ended.iter().map(|s| (s.pr, s.state)).collect(),
            watching: state.prs.iter().map(|(pr, w)| (*pr, w.repo.clone())).collect(),
        }
    }

    /// Latest known status of every watched PR.
    pub fn statuses(&self) -> Vec<Arc<PrSnapshot>> {
        self.lock().prs.values().filter_map(|w| w.status.clone()).collect()
    }

    async fn run(self: Arc<Self>) {
        loop {
            let due = {
                let mut state = self.lock();
                let idle = state.last_touch.is_none_or(|t| t.elapsed() > IDLE_TTL)
                    && !self.blocking.load(Ordering::SeqCst);
                if state.prs.is_empty() || idle {
                    if idle && !state.prs.is_empty() {
                        tracing::info!(prs = ?state.prs.keys().collect::<Vec<_>>(), "pr watch idle; stopping");
                        state.prs.clear();
                    }
                    state.running = false;
                    return;
                }
                state.prs.iter().map(|(pr, w)| (w.next_poll, *pr)).min()
            };
            let Some((at, number)) = due else { continue };

            if at > Instant::now() {
                tokio::select! {
                    () = tokio::time::sleep_until(at) => {}
                    () = self.poke.notified() => {}
                }
                continue;
            }

            let source = self.lock().prs.get_mut(&number).and_then(|w| w.source.take());
            let Some(mut source) = source else { continue };
            let polled = source.poll(&self.gh, &self.budget).await;
            self.record(number, source, polled);
        }
    }

    fn record(&self, number: u64, source: GithubPr, polled: Result<Polled>) {
        let blocking = self.blocking.load(Ordering::SeqCst);
        let mut state = self.lock();
        let Some(watched) = state.prs.get_mut(&number) else { return };
        let now = Instant::now();
        watched.last_poll = Some(now);

        let mut delivered = Vec::new();
        let mut ended = false;
        match polled {
            Ok(polled) => {
                watched.error = polled.warning;
                let moved = polled.changed || !polled.events.is_empty();
                watched.quiet = if moved { 0 } else { watched.quiet.saturating_add(1) };
                let pending = polled.snapshot.checks.as_ref().is_some_and(|c| c.counts.pending > 0);
                watched.status = Some(polled.snapshot);
                for event in polled.events {
                    ended |= event.ends_watch();
                    let event = Arc::new(event);
                    if event.wanted(watched.until) {
                        delivered.push(PrEvent { pr: number, event: Arc::clone(&event) });
                    }
                    watched.recent.push_back(event);
                    if watched.recent.len() > RECENT {
                        watched.recent.pop_front();
                    }
                }
                watched.next_poll =
                    now + next_delay(watched.quiet, blocking, pending, &source.rate);
            }
            Err(e) if fatal(&e, &source.rate) => {
                tracing::warn!(pr = number, error = %e, "pr watch ended");
                ended = true;
                let event = Arc::new(Event::Error { message: e.to_string() });
                delivered.push(PrEvent { pr: number, event });
            }
            Err(e) => {
                tracing::debug!(pr = number, error = %e, "pr poll failed; backing off");
                watched.error = Some(e.to_string());
                watched.quiet = watched.quiet.saturating_add(1);
                watched.next_poll = now + next_delay(watched.quiet, blocking, false, &source.rate);
            }
        }

        if ended {
            if let Some(status) = state.prs.remove(&number).and_then(|w| w.status) {
                state.ended.push_back(status);
                if state.ended.len() > RECENT {
                    state.ended.pop_front();
                }
            }
        } else if let Some(watched) = state.prs.get_mut(&number) {
            watched.source = Some(source);
        }

        if !delivered.is_empty() {
            state.inbox.extend(delivered);
            self.inbox_version.send_modify(|v| *v += 1);
        }
    }

    /// The repo and number of the checked-out branch's PR in `repo`.
    async fn branch_pr(&self, repo: Option<&str>) -> Result<(crate::workspace::ForgeRepo, u64)> {
        let branch = crate::git::current_branch(self.env.cwd())
            .ok()
            .flatten()
            .ok_or_else(|| CliError::no_target("detached HEAD; pass `pr`"))?;
        let key = format!("{}:{branch}", repo.unwrap_or_default());
        if let Some(found) = self.branch_prs.lock().expect("branch cache lock").get(&key) {
            return Ok(found.clone());
        }

        let found = self.gh.locate_pr(&self.env, repo, None).await?;
        self.branch_prs.lock().expect("branch cache lock").insert(key, found.clone());
        Ok(found)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("pr watch state lock poisoned")
    }
}

impl Watched {
    fn new(repo: String, source: GithubPr, until: EventKinds) -> Self {
        Self {
            repo,
            source: Some(source),
            until,
            status: None,
            recent: VecDeque::new(),
            error: None,
            quiet: 0,
            last_poll: None,
            next_poll: Instant::now(),
        }
    }
}

/// The one blocking `pr_watch` call. Dropping it frees the slot.
pub struct Blocker {
    watcher: Arc<PrWatcher>,
}

impl Blocker {
    /// Wait up to `wait` for events on any watched PR and take them all.
    /// `timed_out` is true when `wait` ran out first.
    pub async fn wait(&self, wait: Duration) -> (Vec<PrEvent>, bool) {
        let watcher = &self.watcher;
        let mut version = watcher.inbox_version.subscribe();
        let deadline = Instant::now() + wait;

        loop {
            version.borrow_and_update();
            {
                let mut state = watcher.lock();
                state.last_touch = Some(Instant::now());
                if !state.inbox.is_empty() {
                    return (state.inbox.drain(..).collect(), false);
                }
                if state.prs.is_empty() {
                    return (Vec::new(), false);
                }
            }
            tokio::select! {
                changed = version.changed() => {
                    if changed.is_err() {
                        return (Vec::new(), false);
                    }
                }
                () = tokio::time::sleep_until(deadline) => return (Vec::new(), true),
            }
        }
    }
}

impl Drop for Blocker {
    fn drop(&mut self) {
        self.watcher.blocking.store(false, Ordering::SeqCst);
    }
}

/// Next poll for one PR: 10s growing 1.5x per quiet poll, capped at 2 min
/// while a call waits or 5 min otherwise; 30s at most while checks run.
/// Rate headers only ever stretch it.
fn next_delay(quiet: u8, blocking: bool, checks_pending: bool, rate: &RateHint) -> Duration {
    let cap_ms = if blocking { 120_000 } else { 300_000 };
    let backoff = ExponentialBackoff::new()
        .base_delay_ms(10_000)
        .multiplier(1.5)
        .max_delay_ms(cap_ms)
        .max_attempts(u8::MAX)
        .jitter_factor(0.2);
    let attempt = quiet.min(u8::MAX - 2) + 1;
    let mut ms = backoff.delay(attempt, &mut chrono_machines::rand::rng()).unwrap_or(cap_ms);

    if checks_pending {
        ms = ms.min(30_000);
    }
    if let Some(secs) = rate.retry_after {
        ms = ms.max(secs * 1000);
    }
    if let (Some(left), Some(reset)) = (rate.remaining, rate.reset)
        && left < LOW_WATER
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        ms = ms.max(reset.saturating_sub(now) * 1000 / (left + 1));
    }
    Duration::from_millis(ms).max(MIN_GAP)
}

/// Errors that retrying will not fix: the PR is gone or out of reach, or the
/// API answered something we cannot read. Rate limiting is not one of them.
fn fatal(error: &CliError, rate: &RateHint) -> bool {
    match error {
        CliError::ExecutionFailed { code: 401 | 404 | 410 | 422, .. } | CliError::Json { .. } => {
            true
        }
        CliError::ExecutionFailed { code: 403, .. } => {
            rate.remaining != Some(0) && rate.retry_after.is_none()
        }
        _ => false,
    }
}
