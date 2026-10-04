//! Pull requests over GraphQL: the PR of a branch, and PR listings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

const OVERVIEW_QUERY: &str = r"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed viewerDefaultMergeMethod
    pullRequest(number: $number) {
      number title state isDraft createdAt updatedAt mergedAt body
      author { login } headRefName baseRefName headRefOid
      mergeable mergeStateStatus reviewDecision
      additions deletions changedFiles
      labels(first: 20) { nodes { name } }
      files(first: 100) { nodes { path additions deletions changeType } }
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

    /// What deciding on a merge takes: state, mergeability, review, allowed
    /// merge methods, labels, files and the body.
    pub async fn pr_overview(&self, repo: &ForgeRepo, number: u64) -> Result<PrOverview> {
        let variables =
            serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
        let data: OverviewData = self.graphql(&repo.host, OVERVIEW_QUERY, variables).await?;
        let mut repository = data
            .repository
            .ok_or_else(|| CliError::no_target(format!("no repository {}", repo.full_name())))?;
        let pull = repository.pull_request.take().ok_or_else(|| {
            CliError::no_target(format!("no pull request #{number} in {}", repo.full_name()))
        })?;
        Ok(pull.into_overview(&repository))
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

/// A PR as needed to decide on merging it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrOverview {
    pub pr: u64,
    pub title: String,
    /// open | closed | merged
    pub state: String,
    pub draft: bool,
    pub author: String,
    pub head: String,
    pub base: String,
    pub head_sha: String,
    /// mergeable | conflicting | unknown
    pub mergeable: String,
    /// clean | blocked | behind | dirty | unstable | draft | has_hooks | unknown
    pub merge_state: String,
    /// approved | changes_requested | review_required; absent without review rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    /// Methods the repo allows: merge | squash | rebase
    pub merge_methods: Vec<String>,
    /// GitHub's default method for this viewer
    pub default_method: String,
    pub labels: Vec<String>,
    pub additions: u64,
    pub deletions: u64,
    /// Total changed files; `files` lists the first 100.
    pub changed_files: u64,
    pub files: Vec<PrFile>,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    /// added | deleted | modified | renamed | copied | changed
    pub change: String,
}

#[derive(Deserialize)]
struct OverviewData {
    repository: Option<OverviewRepo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverviewRepo {
    merge_commit_allowed: bool,
    squash_merge_allowed: bool,
    rebase_merge_allowed: bool,
    viewer_default_merge_method: String,
    pull_request: Option<OverviewPull>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverviewPull {
    number: u64,
    title: String,
    state: String,
    is_draft: bool,
    created_at: String,
    updated_at: String,
    merged_at: Option<String>,
    body: String,
    author: Option<super::User>,
    head_ref_name: String,
    base_ref_name: String,
    head_ref_oid: String,
    mergeable: String,
    merge_state_status: String,
    review_decision: Option<String>,
    additions: u64,
    deletions: u64,
    changed_files: u64,
    labels: Option<Nodes<Label>>,
    files: Option<Nodes<FileNode>>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileNode {
    path: String,
    additions: u64,
    deletions: u64,
    change_type: String,
}

impl OverviewPull {
    fn into_overview(self, repo: &OverviewRepo) -> PrOverview {
        let allowed = [
            ("merge", repo.merge_commit_allowed),
            ("squash", repo.squash_merge_allowed),
            ("rebase", repo.rebase_merge_allowed),
        ];
        PrOverview {
            pr: self.number,
            title: self.title,
            state: self.state.to_lowercase(),
            draft: self.is_draft,
            author: super::login(self.author),
            head: self.head_ref_name,
            base: self.base_ref_name,
            head_sha: self.head_ref_oid,
            mergeable: self.mergeable.to_lowercase(),
            merge_state: self.merge_state_status.to_lowercase(),
            review: self.review_decision.map(|r| r.to_lowercase()),
            merge_methods: allowed
                .iter()
                .filter(|(_, on)| *on)
                .map(|(method, _)| (*method).to_string())
                .collect(),
            default_method: repo.viewer_default_merge_method.to_lowercase(),
            labels: self
                .labels
                .map(|l| l.nodes.into_iter().map(|l| l.name).collect())
                .unwrap_or_default(),
            additions: self.additions,
            deletions: self.deletions,
            changed_files: self.changed_files,
            files: self
                .files
                .map(|f| {
                    f.nodes
                        .into_iter()
                        .map(|f| PrFile {
                            path: f.path,
                            additions: f.additions,
                            deletions: f.deletions,
                            change: f.change_type.to_lowercase(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            body: crate::provider::text::readable(&self.body),
            created_at: self.created_at,
            updated_at: self.updated_at,
            merged_at: self.merged_at,
        }
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
