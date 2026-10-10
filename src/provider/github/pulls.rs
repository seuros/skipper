//! Pull requests over GraphQL: the PR of a branch, and PR listings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::runs::repo_path;
use super::{GitHubProvider, Label, Nodes, PrState, label_names, lowercase, rest_url};
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
      number title state isDraft mergedAt
      author { login } headRefName baseRefName headRefOid
      mergeable mergeStateStatus reviewDecision
      additions deletions changedFiles
      labels(first: 20) { nodes { name } }
    }
  }
}";

const SEARCH_QUERY: &str = r"query($q: String!) {
  search(query: $q, type: ISSUE, first: 30) {
    nodes { ... on PullRequest { number title state mergedAt headRefName } }
  }
}";

impl GitHubProvider {
    /// The PR in `repo` whose head is `head_owner`'s `branch`.
    pub async fn pr_for_branch(
        &self,
        repo: &ForgeRepo,
        head_owner: &str,
        branch: &str,
    ) -> Result<u64> {
        #[derive(Deserialize)]
        struct Data {
            repository: Option<Repository>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repository {
            pull_requests: Nodes<BranchPr>,
        }

        #[derive(Serialize)]
        struct Variables<'a> {
            owner: &'a str,
            name: &'a str,
            branch: &'a str,
        }

        let variables = Variables { owner: &repo.owner, name: &repo.name, branch };
        let data: Data = self.graphql(&repo.host, BRANCH_QUERY, &variables).await?;
        let prs = data
            .repository
            .ok_or_else(|| CliError::no_target(format!("no repository {}", repo.full_name())))?
            .pull_requests
            .nodes;
        pick_branch_pr(head_owner, &prs).ok_or_else(|| {
            CliError::no_target(if prs.is_empty() {
                format!("no pull request for {branch} in {}; pass the PR number", repo.full_name())
            } else {
                format!("several forks have a pull request from {branch}; pass the PR number")
            })
        })
    }

    /// What deciding on a merge takes: state, mergeability, review, the
    /// checks' verdict, labels, size and the merge methods allowed.
    pub async fn pr_overview(&self, repo: &ForgeRepo, number: u64) -> Result<PrOverview> {
        let variables = PrVariables { owner: &repo.owner, name: &repo.name, number };
        let (data, checks) = tokio::join!(
            self.graphql::<OverviewData, _>(&repo.host, OVERVIEW_QUERY, &variables),
            self.pr_checks(repo, number)
        );
        let mut repository = data?
            .repository
            .ok_or_else(|| CliError::no_target(format!("no repository {}", repo.full_name())))?;
        let pull = repository.pull_request.take().ok_or_else(|| {
            CliError::no_target(format!("no pull request #{number} in {}", repo.full_name()))
        })?;
        Ok(pull.into_overview(repository, CheckVerdict::of(checks?)))
    }

    /// The files `number` changes, with their line counts.
    pub async fn pr_files(&self, repo: &ForgeRepo, number: u64) -> Result<Vec<PrFile>> {
        #[derive(Deserialize)]
        struct RestFile {
            filename: String,
            status: String,
            additions: u64,
            deletions: u64,
        }
        let base = repo_path(repo);
        let mut files = Vec::new();
        for page in 1.. {
            let url = rest_url(
                &repo.host,
                format_args!("{base}/pulls/{number}/files?per_page={}&page={page}", super::PAGE),
            );
            let batch: Vec<RestFile> = self.api_json_at(&repo.host, &url).await?;
            let full = batch.len() == super::PAGE;
            files.extend(batch.into_iter().map(|f| PrFile {
                path: f.filename,
                additions: f.additions,
                deletions: f.deletions,
                change: f.status,
            }));
            if !full {
                break;
            }
        }
        Ok(files)
    }

    /// Merge `number` by `method`, provided its head is still `head_sha`: a
    /// push after the caller looked fails the merge instead of merging code
    /// nobody checked. Returns the merge commit's sha.
    pub async fn merge_pr(
        &self,
        repo: &ForgeRepo,
        number: u64,
        method: &str,
        head_sha: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Merged {
            sha: String,
        }
        #[derive(Serialize)]
        struct Merge<'a> {
            merge_method: &'a str,
            sha: &'a str,
        }
        let url = rest_url(&repo.host, format_args!("{}/pulls/{number}/merge", repo_path(repo)));
        let body = Merge { merge_method: method, sha: head_sha };
        let response = super::client::put(&repo.host, &url, &body).await?;
        let merged: Merged = response.ok_json(format_args!("merge #{number}"))?;
        Ok(merged.sha)
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

        #[derive(Serialize)]
        struct Variables {
            q: String,
        }
        let variables = Variables { q: search_query(repo, state, author)? };
        let data: Data = self.graphql(&repo.host, SEARCH_QUERY, &variables).await?;
        Ok(data
            .search
            .nodes
            .into_iter()
            .map(|l| PrState {
                pr: l.number,
                title: l.title,
                head: l.head_ref_name,
                state: lowercase(l.state),
                merged_at: l.merged_at,
            })
            .collect())
    }
}

/// A PR as needed to decide on merging it. Its description is the first note
/// of its discussion; its files have their own read.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrOverview {
    pub pr: u64,
    pub title: String,
    /// open | closed | merged
    pub state: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
    pub author: String,
    pub head: String,
    pub base: String,
    pub head_sha: String,
    /// mergeable | conflicting | unknown
    pub mergeable: String,
    /// clean | blocked | behind | dirty | unstable | draft | `has_hooks` | unknown
    pub merge_state: String,
    /// approved | `changes_requested` | `review_required`; absent without review rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    pub checks: CheckVerdict,
    /// Methods the repo allows: merge | squash | rebase
    pub merge_methods: Vec<&'static str>,
    /// GitHub's default method for this viewer
    pub default_method: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
}

/// The head commit's checks, in a line: the conclusion, the counts that are
/// not zero, and which checks failed.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CheckVerdict {
    /// success | failure | cancelled | pending | `no_checks`
    pub conclusion: &'static str,
    pub counts: super::CheckCounts,
    /// Failed or cancelled checks, `workflow / name`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
}

impl CheckVerdict {
    pub fn of(checks: Vec<super::PrCheck>) -> Self {
        let counts = super::CheckCounts::tally(&checks);
        let failed = checks
            .into_iter()
            .filter(|c| matches!(c.bucket, "fail" | "cancel"))
            .map(|c| {
                if c.workflow.is_empty() {
                    return c.name;
                }
                let mut label = c.workflow;
                label.push_str(" / ");
                label.push_str(&c.name);
                label
            })
            .collect();
        Self { conclusion: counts.conclusion(), counts, failed }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PrFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    /// added | removed | modified | renamed | copied | changed
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
    merged_at: Option<String>,
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
}

/// GraphQL variables naming one PR.
#[derive(Serialize)]
pub(super) struct PrVariables<'a> {
    pub owner: &'a str,
    pub name: &'a str,
    pub number: u64,
}

impl OverviewPull {
    fn into_overview(self, repo: OverviewRepo, checks: CheckVerdict) -> PrOverview {
        let allowed = [
            ("merge", repo.merge_commit_allowed),
            ("squash", repo.squash_merge_allowed),
            ("rebase", repo.rebase_merge_allowed),
        ];
        PrOverview {
            pr: self.number,
            title: self.title,
            state: lowercase(self.state),
            draft: self.is_draft,
            author: super::login(self.author),
            head: self.head_ref_name,
            base: self.base_ref_name,
            head_sha: self.head_ref_oid,
            mergeable: lowercase(self.mergeable),
            merge_state: lowercase(self.merge_state_status),
            review: self.review_decision.map(lowercase),
            checks,
            merge_methods: allowed
                .iter()
                .filter(|(_, on)| *on)
                .map(|(method, _)| *method)
                .collect(),
            default_method: lowercase(repo.viewer_default_merge_method),
            labels: label_names(self.labels),
            additions: self.additions,
            deletions: self.deletions,
            changed_files: self.changed_files,
            merged_at: self.merged_at,
        }
    }
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
    let own = |pr: &&BranchPr| pr.head_repository_owner.as_ref().is_some_and(|o| o.login == owner);
    if let Some(newest) = prs.iter().find(own) {
        return Some(prs.iter().filter(own).find(|pr| pr.state == "OPEN").unwrap_or(newest).number);
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
