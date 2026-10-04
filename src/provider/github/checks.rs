//! A PR's checks from the GraphQL status rollup of its head commit: check
//! runs with their workflow names, and commit statuses, in one query per page.

use std::collections::HashMap;

use serde::Deserialize;

use super::{GitHubProvider, PrCheck, run_bucket, status_bucket};
use crate::error::{CliError, Result};
use crate::workspace::ForgeRepo;

const QUERY: &str = r"query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      commits(last: 1) { nodes { commit { statusCheckRollup {
        contexts(first: 100, after: $after) {
          pageInfo { hasNextPage endCursor }
          nodes {
            __typename
            ... on CheckRun {
              name status conclusion detailsUrl title startedAt
              checkSuite { workflowRun { workflow { name } } }
            }
            ... on StatusContext { context state targetUrl description createdAt }
          }
        }
      } } } }
    }
  }
}";

impl GitHubProvider {
    /// Checks on the head commit of `pr`; empty when none reported.
    pub async fn pr_checks(&self, repo: &ForgeRepo, pr: u64) -> Result<Vec<PrCheck>> {
        let mut contexts = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let variables = serde_json::json!({
                "owner": repo.owner, "name": repo.name, "number": pr, "after": after,
            });
            let data: Data = self.graphql(&repo.host, QUERY, variables).await?;
            let pull = data.repository.and_then(|r| r.pull_request).ok_or_else(|| {
                CliError::no_target(format!("no pull request #{pr} in {}", repo.full_name()))
            })?;
            let Some(page) = pull
                .commits
                .nodes
                .into_iter()
                .next()
                .and_then(|node| node.commit.status_check_rollup)
                .map(|rollup| rollup.contexts)
            else {
                break;
            };
            contexts.extend(page.nodes);
            match page.page_info {
                PageInfo { has_next_page: true, end_cursor: Some(cursor) } => after = Some(cursor),
                _ => break,
            }
        }
        Ok(latest(contexts))
    }
}

/// One check per workflow and name (or status context), the latest kept: a
/// re-run leaves its earlier attempts in the rollup.
pub(super) fn latest(contexts: Vec<Context>) -> Vec<PrCheck> {
    let mut kept: HashMap<(String, String), (Option<String>, PrCheck)> = HashMap::new();
    let mut order = Vec::new();
    for context in contexts {
        let (at, check) = context.into_check();
        let key = (check.workflow.clone(), check.name.clone());
        match kept.get(&key) {
            Some((seen, _)) if *seen >= at => {}
            Some(_) => {
                kept.insert(key, (at, check));
            }
            None => {
                order.push(key.clone());
                kept.insert(key, (at, check));
            }
        }
    }
    order.into_iter().filter_map(|key| kept.remove(&key)).map(|(_, check)| check).collect()
}

#[derive(Deserialize)]
struct Data {
    repository: Option<Repository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    pull_request: Option<Pull>,
}

#[derive(Deserialize)]
struct Pull {
    commits: Nodes<CommitNode>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    contexts: Contexts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contexts {
    page_info: PageInfo,
    nodes: Vec<Context>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
pub(super) enum Context {
    CheckRun(CheckRun),
    StatusContext(StatusContext),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
    details_url: Option<String>,
    title: Option<String>,
    started_at: Option<String>,
    check_suite: Option<CheckSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckSuite {
    workflow_run: Option<WorkflowRunRef>,
}

#[derive(Deserialize)]
struct WorkflowRunRef {
    workflow: Workflow,
}

#[derive(Deserialize)]
struct Workflow {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusContext {
    context: String,
    state: String,
    target_url: Option<String>,
    description: Option<String>,
    created_at: Option<String>,
}

fn present(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

impl Context {
    /// The check, and when it started (for picking the latest attempt).
    fn into_check(self) -> (Option<String>, PrCheck) {
        match self {
            Self::CheckRun(run) => (
                run.started_at,
                PrCheck {
                    bucket: run_bucket(&run.status, run.conclusion.as_deref()).to_string(),
                    workflow: run
                        .check_suite
                        .and_then(|s| s.workflow_run)
                        .map(|w| w.workflow.name)
                        .unwrap_or_default(),
                    name: run.name,
                    link: present(run.details_url),
                    description: present(run.title),
                },
            ),
            Self::StatusContext(status) => (
                status.created_at,
                PrCheck {
                    bucket: status_bucket(&status.state).to_string(),
                    workflow: String::new(),
                    name: status.context,
                    link: present(status.target_url),
                    description: present(status.description),
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests;
