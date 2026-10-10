//! A watched GitHub PR, read over REST. Requests carry the last `ETag`, so an
//! unchanged PR answers 304 and costs no rate limit.

use std::collections::HashSet;
use std::fmt::Display;
use std::sync::Arc;

use serde::Deserialize;

use super::{ApiBudget, ChecksSummary, Event, FailedCheck, PrSnapshot, RateHint};
use crate::error::{CliError, Result};
use crate::provider::github::{
    ApiResponse, CheckCounts, GitHubProvider, PAGE, User, login, rest_url, run_bucket,
    status_bucket,
};
use crate::provider::text::{EVENT_BODY_LIMIT, clip, readable_by};

/// Pages of new comments read per poll before waiting for the next one.
const MAX_PAGES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    /// The REST URL of `tail` under this PR's repo.
    fn url(&self, tail: impl Display) -> String {
        rest_url(&self.host, format_args!("repos/{}/{}/{tail}", self.owner, self.repo))
    }
}

#[derive(Debug, Deserialize)]
struct RestPr {
    state: String,
    merged: bool,
    merged_at: Option<String>,
    merged_by: Option<User>,
    merge_commit_sha: Option<String>,
    closed_at: Option<String>,
    updated_at: String,
    html_url: String,
    comments: u64,
    review_comments: u64,
    head: Head,
}

#[derive(Debug, Deserialize)]
struct Head {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct RestComment {
    id: u64,
    user: Option<User>,
    body: Option<String>,
    html_url: String,
    created_at: String,
    /// Inline review comments only.
    path: Option<String>,
    line: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RestReview {
    id: u64,
    user: Option<User>,
    body: Option<String>,
    state: String,
    html_url: String,
    submitted_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CheckRuns {
    check_runs: Vec<CheckRun>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub html_url: Option<String>,
    pub output: Option<CheckOutput>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CheckOutput {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CombinedStatus {
    statuses: Vec<CommitStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CommitStatus {
    pub context: String,
    pub state: String,
    pub target_url: Option<String>,
    pub description: Option<String>,
}

pub struct Polled {
    pub snapshot: Arc<PrSnapshot>,
    pub events: Vec<Event>,
    /// Something moved, even if no event was worth reporting.
    pub changed: bool,
    /// A partial failure (checks unreadable); the rest of the poll stands.
    pub warning: Option<String>,
}

pub struct GithubPr {
    pub pr: PrRef,
    /// Rate headers from the latest response.
    pub rate: RateHint,
    last: Option<RestPr>,
    pr_etag: Option<String>,
    /// PR `updated_at` when the watch began: older comments and reviews are
    /// history, not news.
    since: String,
    seen_comments: HashSet<u64>,
    seen_reviews: HashSet<u64>,
    runs_etag: Option<String>,
    status_etag: Option<String>,
    runs: Vec<CheckRun>,
    statuses: Vec<CommitStatus>,
    checks: Option<ChecksSummary>,
    /// The last snapshot handed out; dropped whenever the PR or its checks
    /// change, so a quiet poll shares it instead of building another.
    snapshot: Option<Arc<PrSnapshot>>,
}

impl GithubPr {
    pub fn new(pr: PrRef) -> Self {
        Self {
            pr,
            rate: RateHint::default(),
            last: None,
            pr_etag: None,
            since: String::new(),
            seen_comments: HashSet::new(),
            seen_reviews: HashSet::new(),
            runs_etag: None,
            status_etag: None,
            runs: Vec::new(),
            statuses: Vec::new(),
            checks: None,
            snapshot: None,
        }
    }

    pub async fn poll(&mut self, gh: &GitHubProvider, budget: &ApiBudget) -> Result<Polled> {
        let number = self.pr.number;
        let mut events = Vec::new();
        let mut changed = false;

        // All or nothing: the ETag, PR state, and seen ids only move once every
        // follow-up read succeeded. On failure the next poll gets a 200 against
        // the old ETag and rebuilds the same events, so none are lost or doubled.
        let url = self.pr.url(format_args!("pulls/{number}"));
        let etag = self.pr_etag.as_deref();
        if let Some(response) = get(gh, budget, &self.pr.host, &mut self.rate, &url, etag).await? {
            let next: RestPr = response.json()?;
            let mut comments = Vec::new();
            let mut reviews = Vec::new();

            // What moved since the last read, settled before the follow-up
            // reads borrow `self`.
            let moved = match &self.last {
                None => {
                    self.since.clone_from(&next.updated_at);
                    events.extend(ended(&next));
                    None
                }
                Some(prev) => {
                    events.extend(transitions(prev, &next));
                    Some((
                        prev.updated_at != next.updated_at,
                        next.comments != prev.comments,
                        next.review_comments != prev.review_comments,
                    ))
                }
            };
            if let Some((updated, new_comments, new_review_comments)) = moved {
                changed = updated;
                if new_comments {
                    comments.extend(self.comments(gh, budget, "issues").await?);
                }
                if new_review_comments {
                    comments.extend(self.comments(gh, budget, "pulls").await?);
                }
                if changed {
                    reviews = self.reviews(gh, budget).await?;
                }
            }

            self.pr_etag = response.etag;
            self.last = Some(next);
            self.snapshot = None;
            for (id, event) in comments {
                self.seen_comments.insert(id);
                events.push(event);
            }
            for (id, event) in reviews {
                self.seen_reviews.insert(id);
                events.push(event);
            }
        }

        let Some(pr) = &self.last else {
            return Err(CliError::parse_error("github", &url, "no PR data yet"));
        };

        // Checks are read separately: failing them must not drop PR events.
        let mut warning = None;
        if pr.state == "open" {
            let sha = pr.head.sha.clone();
            match self.refresh_checks(gh, budget, sha).await {
                Ok(Some(event)) => {
                    changed = true;
                    events.push(event);
                }
                Ok(None) => {}
                Err(e) => warning = Some(format!("checks: {e}")),
            }
        }

        let snapshot = if let Some(snapshot) = &self.snapshot {
            Arc::clone(snapshot)
        } else {
            let snapshot = Arc::new(self.snapshot_now());
            self.snapshot = Some(Arc::clone(&snapshot));
            snapshot
        };
        Ok(Polled { snapshot, events, changed, warning })
    }

    fn snapshot_now(&self) -> PrSnapshot {
        let pr = self.last.as_ref().expect("snapshot after a successful PR read");
        PrSnapshot {
            pr: self.pr.number,
            url: pr.html_url.clone(),
            state: if pr.merged {
                "merged"
            } else if pr.state == "closed" {
                "closed"
            } else {
                "open"
            },
            head_sha: pr.head.sha.clone(),
            checks: self.checks.clone(),
        }
    }

    /// New issue comments (`issues`) or inline review comments (`pulls`).
    async fn comments(
        &mut self,
        gh: &GitHubProvider,
        budget: &ApiBudget,
        kind: &'static str,
    ) -> Result<Vec<(u64, Event)>> {
        let number = self.pr.number;
        let mut events = Vec::new();
        for page in 1..=MAX_PAGES {
            let url = self.pr.url(format_args!(
                "{kind}/{number}/comments?since={}&per_page={PAGE}&page={page}",
                self.since
            ));
            let Some(response) = get(gh, budget, &self.pr.host, &mut self.rate, &url, None).await?
            else {
                break;
            };
            let batch: Vec<RestComment> = response.json()?;
            let full = batch.len() == PAGE;
            for c in batch {
                if c.created_at > self.since && !self.seen_comments.contains(&c.id) {
                    let author = login(c.user);
                    let body = readable_by(&author, c.body.as_deref().unwrap_or_default());
                    events.push((
                        c.id,
                        Event::Comment {
                            body: clip(body, EVENT_BODY_LIMIT),
                            author,
                            url: c.html_url,
                            at: c.created_at,
                            path: c.path,
                            line: c.line,
                        },
                    ));
                }
            }
            if !full {
                break;
            }
        }
        Ok(events)
    }

    async fn reviews(
        &mut self,
        gh: &GitHubProvider,
        budget: &ApiBudget,
    ) -> Result<Vec<(u64, Event)>> {
        let number = self.pr.number;
        let mut events = Vec::new();
        for page in 1..=MAX_PAGES {
            let url =
                self.pr.url(format_args!("pulls/{number}/reviews?per_page={PAGE}&page={page}"));
            let Some(response) = get(gh, budget, &self.pr.host, &mut self.rate, &url, None).await?
            else {
                break;
            };
            let batch: Vec<RestReview> = response.json()?;
            let full = batch.len() == PAGE;
            for r in batch {
                let fresh = r.submitted_at.as_deref().is_some_and(|at| at > self.since.as_str());
                if fresh && r.state != "PENDING" && !self.seen_reviews.contains(&r.id) {
                    let author = login(r.user);
                    let body = readable_by(&author, r.body.as_deref().unwrap_or_default());
                    events.push((
                        r.id,
                        Event::Review {
                            body: clip(body, EVENT_BODY_LIMIT),
                            author,
                            state: r.state,
                            url: r.html_url,
                            at: r.submitted_at,
                        },
                    ));
                }
            }
            if !full {
                break;
            }
        }
        Ok(events)
    }

    /// Re-read check runs and commit statuses for `sha`; a `Checks` event when
    /// the combined conclusion moves to a settled value.
    async fn refresh_checks(
        &mut self,
        gh: &GitHubProvider,
        budget: &ApiBudget,
        sha: String,
    ) -> Result<Option<Event>> {
        if self.checks.as_ref().is_some_and(|c| c.sha != sha) {
            self.runs_etag = None;
            self.status_etag = None;
            self.checks = None;
        }

        let host = &self.pr.host;
        let url = self.pr.url(format_args!("commits/{sha}/check-runs?per_page={PAGE}"));
        let runs = get(gh, budget, host, &mut self.rate, &url, self.runs_etag.as_deref()).await?;
        let url = self.pr.url(format_args!("commits/{sha}/status?per_page={PAGE}"));
        let statuses =
            get(gh, budget, host, &mut self.rate, &url, self.status_etag.as_deref()).await?;

        if runs.is_none() && statuses.is_none() && self.checks.is_some() {
            return Ok(None);
        }
        // An ETag is kept only once its body parsed, so a bad body is read
        // again instead of hiding behind a 304.
        if let Some(response) = runs {
            self.runs = response.json::<CheckRuns>()?.check_runs;
            self.runs_etag = response.etag;
        }
        if let Some(response) = statuses {
            self.statuses = response.json::<CombinedStatus>()?.statuses;
            self.status_etag = response.etag;
        }

        let summary = summarize(sha, &self.runs, &self.statuses);
        let moved = self.checks.as_ref().is_none_or(|prev| prev.conclusion != summary.conclusion);
        let settled = !matches!(summary.conclusion, "pending" | "no_checks");
        let event = (moved && settled).then(|| Event::Checks(summary.clone()));
        self.checks = Some(summary);
        self.snapshot = None;
        Ok(event)
    }
}

/// `Some` on 200, `None` on 304; other statuses are errors. Takes the fields
/// it touches rather than the whole PR, so callers lend their `ETag`s.
async fn get(
    gh: &GitHubProvider,
    budget: &ApiBudget,
    host: &str,
    rate: &mut RateHint,
    url: &str,
    etag: Option<&str>,
) -> Result<Option<ApiResponse>> {
    budget.acquire().await;
    let response = gh.api_get(host, url, etag).await?;
    *rate = RateHint {
        remaining: response.rate_remaining,
        reset: response.rate_reset,
        retry_after: response.retry_after,
    };
    match response.status {
        200 => Ok(Some(response)),
        304 => Ok(None),
        _ => Err(response.error(format_args!("GET {url}"))),
    }
}

/// Events between two reads of an open PR.
fn transitions(prev: &RestPr, next: &RestPr) -> Vec<Event> {
    let mut events = Vec::new();
    if prev.head.sha != next.head.sha {
        events.push(Event::Push { sha: next.head.sha.clone() });
    }
    if prev.state == "open" {
        events.extend(ended(next));
    }
    events
}

/// `Merged` or `Closed` when the PR is no longer open.
fn ended(pr: &RestPr) -> Option<Event> {
    if pr.merged {
        Some(Event::Merged {
            by: pr.merged_by.as_ref().map(|u| u.login.clone()),
            sha: pr.merge_commit_sha.clone(),
            at: pr.merged_at.clone(),
        })
    } else if pr.state == "closed" {
        Some(Event::Closed { at: pr.closed_at.clone() })
    } else {
        None
    }
}

pub(super) fn summarize(
    sha: String,
    runs: &[CheckRun],
    statuses: &[CommitStatus],
) -> ChecksSummary {
    let mut counts = CheckCounts::default();
    let mut failed = Vec::new();

    for run in runs {
        let bucket = run_bucket(&run.status, run.conclusion.as_deref());
        counts.add(bucket);
        if bucket == "fail" {
            failed.push(FailedCheck {
                name: run.name.clone(),
                link: run.html_url.clone(),
                description: run.output.as_ref().and_then(|o| o.title.clone()),
            });
        }
    }
    for status in statuses {
        let bucket = status_bucket(&status.state);
        counts.add(bucket);
        if bucket == "fail" {
            failed.push(FailedCheck {
                name: status.context.clone(),
                link: status.target_url.clone(),
                description: status.description.clone(),
            });
        }
    }

    ChecksSummary { sha, conclusion: counts.conclusion(), counts, failed }
}

#[cfg(test)]
mod tests;
