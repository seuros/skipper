//! Pull requests over GraphQL: the PR of a branch, and PR listings.

use serde::Deserialize;

use super::{GitHubProvider, PrState};
use crate::error::{CliError, Result};
use crate::workspace::ForgeRepo;

const BRANCH_QUERY: &str = r"query($owner: String!, $name: String!, $branch: String!) {
  repository(owner: $owner, name: $name) {
    pullRequests(headRefName: $branch, first: 20, orderBy: {field: CREATED_AT, direction: DESC}) {
      nodes { number state headRepositoryOwner { login } }
    }
  }
}";

const SEARCH_QUERY: &str = r"query($q: String!) {
  search(query: $q, type: ISSUE, first: 30) {
    nodes { ... on PullRequest { number title state mergedAt headRefName } }
  }
}";

impl GitHubProvider {
    /// The PR whose head is `branch`.
    pub async fn pr_for_branch(&self, repo: &ForgeRepo, branch: &str) -> Result<u64> {
        #[derive(Deserialize)]
        struct Data {
            repository: Option<Repository>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repository {
            pull_requests: Nodes<BranchPr>,
        }

        let variables =
            serde_json::json!({ "owner": repo.owner, "name": repo.name, "branch": branch });
        let data: Data = self.graphql(&repo.host, BRANCH_QUERY, variables).await?;
        let prs = data
            .repository
            .ok_or_else(|| CliError::no_target(format!("no repository {}", repo.full_name())))?
            .pull_requests
            .nodes;
        pick_branch_pr(&repo.owner, &prs).ok_or_else(|| {
            CliError::no_target(if prs.is_empty() {
                format!("no pull request for {branch} in {}; pass the PR number", repo.full_name())
            } else {
                format!("several forks have a pull request from {branch}; pass the PR number")
            })
        })
    }

    /// The 30 newest PRs of `repo` in `state` (open | closed | merged | all) by
    /// `author` (a login, or `me`).
    pub async fn pr_list(
        &self,
        repo: &ForgeRepo,
        state: &str,
        author: &str,
    ) -> Result<Vec<PrState>> {
        #[derive(Deserialize)]
        struct Data {
            search: Nodes<Listed>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Listed {
            number: u64,
            title: String,
            state: String,
            merged_at: Option<String>,
            head_ref_name: String,
        }

        let query = search_query(repo, state, author)?;
        let data: Data =
            self.graphql(&repo.host, SEARCH_QUERY, serde_json::json!({ "q": query })).await?;
        Ok(data
            .search
            .nodes
            .into_iter()
            .map(|l| PrState {
                pr: l.number,
                title: l.title,
                head: l.head_ref_name,
                state: l.state.to_lowercase(),
                merged_at: l.merged_at,
            })
            .collect())
    }
}

#[derive(Deserialize)]
pub(super) struct Nodes<T> {
    pub nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BranchPr {
    pub number: u64,
    pub state: String,
    pub head_repository_owner: Option<Owner>,
}

#[derive(Deserialize)]
pub(super) struct Owner {
    pub login: String,
}

/// A PR from this repo's own branch, open first, else the newest; else the one
/// fork PR with that branch name. Several fork PRs leave it open.
pub(super) fn pick_branch_pr(owner: &str, prs: &[BranchPr]) -> Option<u64> {
    let own: Vec<&BranchPr> = prs
        .iter()
        .filter(|pr| pr.head_repository_owner.as_ref().is_some_and(|o| o.login == owner))
        .collect();
    if let Some(first) = own.first() {
        return Some(own.iter().find(|pr| pr.state == "OPEN").unwrap_or(first).number);
    }
    match prs {
        [only] => Some(only.number),
        _ => None,
    }
}

/// Search qualifiers for a PR listing. `author` is checked to be a login so
/// it cannot smuggle in other qualifiers (another `repo:`).
pub(super) fn search_query(repo: &ForgeRepo, state: &str, author: &str) -> Result<String> {
    let author = match author {
        "me" => "@me",
        login
            if !login.is_empty()
                && login.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '[' | ']')
                }) =>
        {
            login
        }
        other => return Err(CliError::no_target(format!("not a GitHub login: {other:?}"))),
    };
    let state = match state {
        "open" => " is:open",
        "closed" => " is:closed is:unmerged",
        "merged" => " is:merged",
        _ => "",
    };
    Ok(format!("repo:{}/{} is:pr author:{author}{state} sort:created-desc", repo.owner, repo.name))
}

#[cfg(test)]
mod tests;
