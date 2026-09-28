//! A watched GitHub PR, read over REST through `gh api`. Requests carry the
//! last ETag, so an unchanged PR answers 304 and costs no rate limit.

use std::collections::HashSet;

use serde::Deserialize;

use super::{ApiBudget, ChecksSummary, Event, FailedCheck, PrSnapshot, RateHint};
use crate::error::{CliError, Result};
use crate::provider::github::{ApiResponse, CheckCounts, GitHubProvider};

/// Comment and review bodies are cut past this; the url has the rest.
const BODY_LIMIT: usize = 1500;
const PAGE: usize = 100;
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
    fn path(&self, tail: &str) -> String {
        format!("repos/{}/{}/{tail}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
struct Head {
    sha: String,
}

#[derive(Debug, Clone, Deserialize)]
struct User {
    login: String,
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
    pub snapshot: PrSnapshot,
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
        }
    }

    pub async fn poll(&mut self, gh: &GitHubProvider, budget: &ApiBudget) -> Result<Polled> {
        let number = self.pr.number;
        let mut events = Vec::new();
        let mut changed = false;

        // All or nothing: the ETag, PR state, and seen ids only move once every
        // follow-up read succeeded. On failure the next poll gets a 200 against
        // the old ETag and rebuilds the same events, so none are lost or doubled.
        let path = self.pr.path(&format!("pulls/{number}"));
        let etag = self.pr_etag.clone();
        if let Some(response) = self.get(gh, budget, &path, etag.as_deref()).await? {
            let next: RestPr = response.json()?;
            let mut comments = Vec::new();
            let mut reviews = Vec::new();

            match self.last.clone() {
                None => {
                    self.since = next.updated_at.clone();
                    events.extend(ended(&next));
                }
                Some(prev) => {
                    changed = prev.updated_at != next.updated_at;
                    events.extend(transitions(&prev, &next));
                    if next.comments != prev.comments {
                        comments
                            .extend(self.comments(gh, budget, &format!("issues/{number}")).await?);
                    }
                    if next.review_comments != prev.review_comments {
                        comments
                            .extend(self.comments(gh, budget, &format!("pulls/{number}")).await?);
                    }
                    if changed {
                        reviews = self.reviews(gh, budget).await?;
                    }
                }
            }

            self.pr_etag = response.etag;
            self.last = Some(next);
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
            return Err(CliError::parse_error("gh", &path, "no PR data yet"));
        };

        // Checks are read separately: failing them must not drop PR events.
        let mut warning = None;
        if pr.state == "open" {
            let sha = pr.head.sha.clone();
            match self.refresh_checks(gh, budget, &sha).await {
                Ok(Some(event)) => {
                    changed = true;
                    events.push(event);
                }
                Ok(None) => {}
                Err(e) => warning = Some(format!("checks: {e}")),
            }
        }

        Ok(Polled { snapshot: self.snapshot(), events, changed, warning })
    }

    fn snapshot(&self) -> PrSnapshot {
        let pr = self.last.as_ref().expect("snapshot after a successful PR read");
        PrSnapshot {
            pr: self.pr.number,
            url: pr.html_url.clone(),
            state: if pr.merged { "merged".to_string() } else { pr.state.clone() },
            head_sha: pr.head.sha.clone(),
            checks: self.checks.clone(),
        }
    }

    /// New issue comments (`issues/N`) or inline review comments (`pulls/N`).
    async fn comments(
        &mut self,
        gh: &GitHubProvider,
        budget: &ApiBudget,
        owner: &str,
    ) -> Result<Vec<(u64, Event)>> {
        let mut events = Vec::new();
        for page in 1..=MAX_PAGES {
            let tail = format!("{owner}/comments?since={}&per_page={PAGE}&page={page}", self.since);
            let Some(response) = self.get(gh, budget, &self.pr.path(&tail), None).await? else {
                break;
            };
            let batch: Vec<RestComment> = response.json()?;
            let full = batch.len() == PAGE;
            for c in batch {
                if c.created_at > self.since && !self.seen_comments.contains(&c.id) {
                    events.push((
                        c.id,
                        Event::Comment {
                            author: login(c.user),
                            body: clip(c.body),
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
        let mut events = Vec::new();
        for page in 1..=MAX_PAGES {
            let tail = format!("pulls/{}/reviews?per_page={PAGE}&page={page}", self.pr.number);
            let Some(response) = self.get(gh, budget, &self.pr.path(&tail), None).await? else {
                break;
            };
            let batch: Vec<RestReview> = response.json()?;
            let full = batch.len() == PAGE;
            for r in batch {
                let fresh = r.submitted_at.as_deref().is_some_and(|at| at > self.since.as_str());
                if fresh && r.state != "PENDING" && !self.seen_reviews.contains(&r.id) {
                    events.push((
                        r.id,
                        Event::Review {
                            author: login(r.user),
                            state: r.state,
                            body: clip(r.body),
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
        sha: &str,
    ) -> Result<Option<Event>> {
        if self.checks.as_ref().is_some_and(|c| c.sha != sha) {
            self.runs_etag = None;
            self.status_etag = None;
            self.checks = None;
        }

        let path = self.pr.path(&format!("commits/{sha}/check-runs?per_page={PAGE}"));
        let etag = self.runs_etag.clone();
        let runs = self.get(gh, budget, &path, etag.as_deref()).await?;
        let path = self.pr.path(&format!("commits/{sha}/status?per_page={PAGE}"));
        let etag = self.status_etag.clone();
        let statuses = self.get(gh, budget, &path, etag.as_deref()).await?;

        if runs.is_none() && statuses.is_none() && self.checks.is_some() {
            return Ok(None);
        }
        if let Some(response) = runs {
            self.runs_etag = response.etag.clone();
            self.runs = response.json::<CheckRuns>()?.check_runs;
        }
        if let Some(response) = statuses {
            self.status_etag = response.etag.clone();
            self.statuses = response.json::<CombinedStatus>()?.statuses;
        }

        let summary = summarize(sha, &self.runs, &self.statuses);
        let moved = self.checks.as_ref().is_none_or(|prev| prev.conclusion != summary.conclusion);
        let settled = !matches!(summary.conclusion.as_str(), "pending" | "no_checks");
        self.checks = Some(summary.clone());
        Ok((moved && settled).then_some(Event::Checks(summary)))
    }

    /// `Some` on 200, `None` on 304; other statuses are errors.
    async fn get(
        &mut self,
        gh: &GitHubProvider,
        budget: &ApiBudget,
        path: &str,
        etag: Option<&str>,
    ) -> Result<Option<ApiResponse>> {
        budget.acquire().await;
        let response = gh.api_get(&self.pr.host, path, etag).await?;
        self.rate = RateHint {
            remaining: response.rate_remaining,
            reset: response.rate_reset,
            retry_after: response.retry_after,
        };
        match response.status {
            200 => Ok(Some(response)),
            304 => Ok(None),
            status => Err(CliError::execution_failed(
                "gh",
                i32::from(status),
                format!("HTTP {status} GET {path}"),
            )),
        }
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
            by: pr.merged_by.clone().map(|u| u.login),
            sha: pr.merge_commit_sha.clone(),
            at: pr.merged_at.clone(),
        })
    } else if pr.state == "closed" {
        Some(Event::Closed { at: pr.closed_at.clone() })
    } else {
        None
    }
}

pub(super) fn summarize(sha: &str, runs: &[CheckRun], statuses: &[CommitStatus]) -> ChecksSummary {
    let mut counts = CheckCounts::default();
    let mut failed = Vec::new();

    for run in runs {
        match (run.status.as_str(), run.conclusion.as_deref()) {
            ("completed", Some("success" | "neutral")) => counts.pass += 1,
            ("completed", Some("skipped")) => counts.skipped += 1,
            ("completed", Some("cancelled")) => counts.cancelled += 1,
            ("completed", _) => {
                counts.fail += 1;
                failed.push(FailedCheck {
                    name: run.name.clone(),
                    link: run.html_url.clone(),
                    description: run.output.as_ref().and_then(|o| o.title.clone()),
                });
            }
            _ => counts.pending += 1,
        }
    }
    for status in statuses {
        match status.state.as_str() {
            "success" => counts.pass += 1,
            "pending" => counts.pending += 1,
            _ => {
                counts.fail += 1;
                failed.push(FailedCheck {
                    name: status.context.clone(),
                    link: status.target_url.clone(),
                    description: status.description.clone(),
                });
            }
        }
    }

    ChecksSummary {
        sha: sha.to_string(),
        conclusion: counts.conclusion().to_string(),
        counts,
        failed,
    }
}

fn login(user: Option<User>) -> String {
    user.map_or_else(|| "ghost".to_string(), |u| u.login)
}

fn clip(body: Option<String>) -> String {
    let body = body.unwrap_or_default();
    match body.char_indices().nth(BODY_LIMIT) {
        Some((cut, _)) => format!("{}…", &body[..cut]),
        None => body,
    }
}

#[cfg(test)]
mod tests;
