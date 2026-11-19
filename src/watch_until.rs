//! A chain of forge conditions waited on in order: a PR opening, a PR
//! merging, a workflow run finishing. A step may wait for something that does
//! not exist yet (the release PR a push will open, the build its merge will
//! start), which a watch keyed by a PR number or a run id cannot express.

use std::time::Duration;

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::error::{CliError, Result};
use crate::provider::github::{DatedRun, OpenPr, PrState};
use crate::provider::{BuildRun, retryable};
use crate::tracing;

/// Steps one chain may hold.
pub const MAX_STEPS: usize = 6;
/// Poll this often while a step is fresh, then at `SLOW_POLL`.
const FAST_POLL: Duration = Duration::from_secs(15);
const SLOW_POLL: Duration = Duration::from_secs(30);
const FAST_FOR: Duration = Duration::from_mins(5);
/// A run may be stamped slightly before the event that started it.
const SKEW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "wait_for", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// An open PR matching every filter given (at least one). It need not
    /// exist yet. Later steps use it.
    PrOpened {
        /// Title contains this (case-insensitive)
        title_contains: Option<String>,
        /// Head branch starts with this
        head_prefix: Option<String>,
        /// Title or description contains this, e.g. a short commit sha
        mentions: Option<String>,
    },
    /// A PR is merged. Closed without a merge fails the chain.
    PrMerged {
        /// PR number (default: the PR an earlier `pr_opened` step found)
        pr: Option<u64>,
    },
    /// A workflow run finishes; anything but success fails the chain. After
    /// another step, the first run created since that step's event is the
    /// one. As the first step, the newest run; without `branch` or `commit`,
    /// runs that had finished before the call are ignored. A tag or branch
    /// keeps the runs of every commit it ever pointed at: pass `commit` to
    /// wait for the run of one push, which need not have started yet.
    RunFinished {
        /// Workflow name contains this (case-insensitive)
        workflow: Option<String>,
        /// The run's branch or tag, exactly
        branch: Option<String>,
        /// The run's commit: a full sha or a prefix of 7 or more characters
        commit: Option<String>,
    },
}

impl Step {
    const fn name(&self) -> &'static str {
        match self {
            Self::PrOpened { .. } => "pr_opened",
            Self::PrMerged { .. } => "pr_merged",
            Self::RunFinished { .. } => "run_finished",
        }
    }
}

/// What is wrong with `steps`, for the caller to fix.
pub fn validate(steps: &[Step]) -> std::result::Result<(), String> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("pass 1 to {MAX_STEPS} steps"));
    }
    let mut pr_known = false;
    for (index, step) in steps.iter().enumerate() {
        match step {
            Step::PrOpened { title_contains, head_prefix, mentions } => {
                if [title_contains, head_prefix, mentions].iter().all(|f| blank(f.as_deref())) {
                    return Err(format!(
                        "step {index} (pr_opened) needs title_contains, head_prefix or mentions"
                    ));
                }
                pr_known = true;
            }
            Step::PrMerged { pr } => {
                if pr.is_none() && !pr_known {
                    return Err(format!(
                        "step {index} (pr_merged) needs `pr`, or a pr_opened step before it"
                    ));
                }
                pr_known = true;
            }
            Step::RunFinished { workflow, branch, commit } => {
                if [workflow, branch, commit].iter().all(|f| blank(f.as_deref())) {
                    return Err(format!(
                        "step {index} (run_finished) needs workflow, branch or commit"
                    ));
                }
                if commit.as_deref().is_some_and(|sha| !is_sha_prefix(sha.trim())) {
                    return Err(format!(
                        "step {index} (run_finished) commit must be 7 to 40 hex characters"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_sha_prefix(sha: &str) -> bool {
    (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn blank(value: Option<&str>) -> bool {
    value.is_none_or(|v| v.trim().is_empty())
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StepOutcome {
    /// `pr_opened` | `pr_merged` | `run_finished`
    pub wait_for: &'static str,
    /// done | failed | waiting | `not_reached`
    pub state: &'static str,
    /// The PR the step matched or is waiting on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<PrState>,
    /// The run the step matched; absent while none has appeared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<BuildRun>,
    /// When the step's event happened (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

impl StepOutcome {
    const fn blank(step: &Step, state: &'static str) -> Self {
        Self { wait_for: step.name(), state, pr: None, run: None, at: None }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Outcome {
    /// One entry per step, in order.
    pub steps: Vec<StepOutcome>,
    /// Every step is done.
    pub completed: bool,
    /// A step failed (PR closed unmerged, run not successful); later steps
    /// were not reached.
    pub failed: bool,
    /// `wait_secs` ran out on the step in state `waiting`. Call again with
    /// the steps not yet done, pinning `pr` and `branch` from this result.
    pub expired: bool,
    /// The forge refused a read; the steps show how far the chain got.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub waited_secs: u64,
}

/// The forge reads a chain polls.
pub trait Forge: Sync {
    fn open_prs(&self) -> impl Future<Output = Result<Vec<OpenPr>>> + Send;
    fn pr_state(&self, number: u64) -> impl Future<Output = Result<PrState>> + Send;
    fn runs(&self, branch: Option<&str>) -> impl Future<Output = Result<Vec<DatedRun>>> + Send;
}

enum Progress {
    Done(StepOutcome),
    Failed(StepOutcome),
    Waiting(StepOutcome),
}

/// What earlier steps established for later ones.
struct Carried {
    started: Timestamp,
    pr: Option<u64>,
    /// When the previous step's event happened.
    since: Option<Timestamp>,
    first: bool,
    /// The run the current step settled on; it stays on it while newer
    /// matching runs appear.
    run: Option<String>,
}

/// Wait for `steps` in order, up to `wait` in total. `started` is the wall
/// clock at the call.
pub async fn run<F: Forge>(
    forge: &F,
    steps: &[Step],
    wait: Duration,
    started: Timestamp,
) -> Outcome {
    let begun = Instant::now();
    let deadline = begun + wait;
    let mut carried = Carried { started, pr: None, since: None, first: true, run: None };
    let mut outcome = Outcome {
        steps: Vec::with_capacity(steps.len()),
        completed: false,
        failed: false,
        expired: false,
        error: None,
        waited_secs: 0,
    };

    'steps: for step in steps {
        let step_begun = Instant::now();
        let mut waiting = StepOutcome::blank(step, "waiting");
        loop {
            match poll(forge, step, &carried).await {
                Ok(Progress::Done(done)) => {
                    carried.pr = done.pr.as_ref().map(|pr| pr.pr).or(carried.pr);
                    carried.since =
                        Some(done.at.as_deref().and_then(parse).unwrap_or_else(Timestamp::now));
                    carried.first = false;
                    carried.run = None;
                    outcome.steps.push(done);
                    continue 'steps;
                }
                Ok(Progress::Failed(failed)) => {
                    outcome.steps.push(failed);
                    outcome.failed = true;
                    break 'steps;
                }
                Ok(Progress::Waiting(current)) => {
                    carried.run = current.run.as_ref().map(|run| run.id.clone()).or(carried.run);
                    waiting = current;
                }
                Err(e) if matches!(e, CliError::Timeout { .. }) || retryable(&e) => {
                    tracing::debug!(error = %e, "chain poll failed transiently; polling again");
                }
                Err(e) => {
                    outcome.steps.push(waiting);
                    outcome.error = Some(e.to_string());
                    break 'steps;
                }
            }

            let now = Instant::now();
            if now >= deadline {
                outcome.steps.push(waiting);
                outcome.expired = true;
                break 'steps;
            }
            let interval = if now - step_begun < FAST_FOR { FAST_POLL } else { SLOW_POLL };
            tokio::time::sleep_until((now + interval).min(deadline)).await;
        }
    }

    outcome.completed =
        outcome.steps.len() == steps.len() && outcome.steps.iter().all(|step| step.state == "done");
    for step in &steps[outcome.steps.len()..] {
        outcome.steps.push(StepOutcome::blank(step, "not_reached"));
    }
    outcome.waited_secs = begun.elapsed().as_secs();
    outcome
}

async fn poll<F: Forge>(forge: &F, step: &Step, carried: &Carried) -> Result<Progress> {
    match step {
        Step::PrOpened { title_contains, head_prefix, mentions } => {
            let found = forge.open_prs().await?.into_iter().find(|open| {
                title_contains.as_deref().is_none_or(|t| contains(&open.pr.title, t))
                    && head_prefix.as_deref().is_none_or(|h| open.pr.head.starts_with(h))
                    && mentions
                        .as_deref()
                        .is_none_or(|m| contains(&open.pr.title, m) || contains(&open.body, m))
            });
            Ok(match found {
                Some(open) => Progress::Done(StepOutcome {
                    pr: Some(open.pr),
                    at: Some(open.created_at),
                    ..StepOutcome::blank(step, "done")
                }),
                None => Progress::Waiting(StepOutcome::blank(step, "waiting")),
            })
        }
        Step::PrMerged { pr } => {
            let number =
                pr.or(carried.pr).ok_or_else(|| CliError::no_target("no PR to wait on"))?;
            let pr = forge.pr_state(number).await?;
            let at = pr.merged_at.clone();
            Ok(match pr.state.as_str() {
                "merged" => Progress::Done(StepOutcome {
                    pr: Some(pr),
                    at,
                    ..StepOutcome::blank(step, "done")
                }),
                "closed" => Progress::Failed(StepOutcome {
                    pr: Some(pr),
                    ..StepOutcome::blank(step, "failed")
                }),
                _ => Progress::Waiting(StepOutcome {
                    pr: Some(pr),
                    ..StepOutcome::blank(step, "waiting")
                }),
            })
        }
        Step::RunFinished { workflow, branch, commit } => {
            let branch = branch.as_deref().filter(|b| !b.trim().is_empty());
            let commit = commit.as_deref().map(str::trim).filter(|c| !c.is_empty());
            let mut runs = forge.runs(branch).await?;
            let settled = carried
                .run
                .as_deref()
                .and_then(|id| runs.iter().position(|dated| dated.run.id == id))
                .map(|at| runs.swap_remove(at));
            let mut matching = runs.into_iter().filter(|dated| {
                workflow.as_deref().is_none_or(|w| {
                    dated.run.workflow.as_deref().is_some_and(|name| contains(name, w))
                }) && commit.is_none_or(|sha| {
                    dated.head_sha.as_deref().is_some_and(|head| {
                        head.len() >= sha.len() && head[..sha.len()].eq_ignore_ascii_case(sha)
                    })
                }) && (commit.is_some() || eligible(dated, branch.is_some(), carried))
            });
            // Runs come newest first. The run an earlier step's event started
            // is the oldest since it; a later release may already have its own.
            let found = settled
                .or_else(|| if carried.first { matching.next() } else { matching.next_back() });
            let Some(dated) = found else {
                return Ok(Progress::Waiting(StepOutcome::blank(step, "waiting")));
            };
            let run = dated.run;
            Ok(match run.status {
                "success" | "skipped" | "completed" => Progress::Done(StepOutcome {
                    run: Some(run),
                    at: Some(Timestamp::now().to_string()),
                    ..StepOutcome::blank(step, "done")
                }),
                "failure" | "cancelled" => Progress::Failed(StepOutcome {
                    run: Some(run),
                    ..StepOutcome::blank(step, "failed")
                }),
                _ => Progress::Waiting(StepOutcome {
                    run: Some(run),
                    ..StepOutcome::blank(step, "waiting")
                }),
            })
        }
    }
}

/// Whether `dated` is a run this step may take, rather than an older one of
/// the same workflow.
fn eligible(dated: &DatedRun, pinned: bool, carried: &Carried) -> bool {
    let created = dated.created_at.as_deref().and_then(parse);
    if !carried.first {
        let since = carried.since.unwrap_or(carried.started);
        return created.is_some_and(|at| at + SKEW >= since);
    }
    pinned || !dated.run.is_terminal() || created.is_some_and(|at| at >= carried.started)
}

fn parse(at: &str) -> Option<Timestamp> {
    at.parse().ok()
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests;
