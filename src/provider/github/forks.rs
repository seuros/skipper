//! PR lookup, with forks named in its errors. GitHub's parent link is
//! history, not intent: a project can outgrow the repo it was forked from, so
//! reads never follow it. A PR missing from a fork says what the parent is,
//! and the caller picks with `repo=`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use serde::Deserialize;

use super::GitHubProvider;
use crate::error::{CliError, Result};
use crate::workspace::ForgeRepo;

const PARENT: &str = r"query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) { parent { nameWithOwner } }
}";

const PR_EXISTS: &str = r"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) { number } }
}";

static PARENTS: LazyLock<Mutex<HashMap<String, Option<String>>>> = LazyLock::new(Mutex::default);

impl GitHubProvider {
    /// The repo a PR read goes to (`spec`: a remote name or `owner/name`, else
    /// the workspace's GitHub repo) and the PR there: `pr`, or the one from the
    /// checked-out branch. That branch is pushed to the workspace's repo, so
    /// its owner is the head owner even when reading another repo
    /// (`repo=upstream` from a fork).
    pub async fn locate_pr(
        &self,
        env: &crate::environment::SkipperEnvironment,
        spec: Option<&str>,
        pr: Option<u64>,
    ) -> Result<(ForgeRepo, u64)> {
        use crate::environment::Environment as _;

        let repo = crate::workspace::select_on(env, "github", spec)?;
        let head_owner = crate::workspace::forge_repo_on(env, "github")
            .map_or_else(|_| repo.owner.clone(), |default| default.owner);
        let number = self.resolve_pr(&repo, &head_owner, pr, env.cwd()).await?;
        Ok((repo, number))
    }

    /// PR `pr` in `repo`, or the PR there from `head_owner`'s checked-out
    /// branch (`head_owner` differs from `repo`'s owner when reading a fork's
    /// PR on the repo it targets).
    pub async fn resolve_pr(
        &self,
        repo: &ForgeRepo,
        head_owner: &str,
        pr: Option<u64>,
        cwd: &Path,
    ) -> Result<u64> {
        if let Some(number) = pr {
            if self.pr_exists(repo, number).await? {
                return Ok(number);
            }
            let message = format!("no pull request #{number} in {}", repo.full_name());
            return Err(self.not_found(repo, message).await);
        }
        let branch = crate::git::repo_info(cwd)
            .ok()
            .and_then(|info| info.branch)
            .ok_or_else(|| CliError::no_target("detached HEAD; pass the PR number"))?;
        match self.pr_for_branch(repo, head_owner, &branch).await {
            Err(CliError::NoTarget(message)) => Err(self.not_found(repo, message).await),
            found => found,
        }
    }

    async fn pr_exists(&self, repo: &ForgeRepo, number: u64) -> Result<bool> {
        #[derive(Deserialize)]
        struct Data {
            repository: Option<Repository>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repository {
            pull_request: Option<serde_json::Value>,
        }

        let variables =
            serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
        match self.graphql::<Data>(&repo.host, PR_EXISTS, variables).await {
            Ok(data) => Ok(data.repository.and_then(|r| r.pull_request).is_some()),
            Err(CliError::ExecutionFailed { code: 200, .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `message`, naming `repo`'s parent when it is a fork.
    async fn not_found(&self, repo: &ForgeRepo, message: String) -> CliError {
        let parent = self.parent(repo).await;
        CliError::no_target(fork_hint(message, &repo.full_name(), parent.as_deref()))
    }

    /// `repo`'s parent, once per process; `None` when it has none or GitHub
    /// cannot say (the error it decorates matters more).
    async fn parent(&self, repo: &ForgeRepo) -> Option<String> {
        #[derive(Deserialize)]
        struct Data {
            repository: Option<Repository>,
        }
        #[derive(Deserialize)]
        struct Repository {
            parent: Option<Parent>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Parent {
            name_with_owner: String,
        }

        let key = format!("{}/{}", repo.host, repo.full_name());
        if let Some(parent) = PARENTS.lock().expect("parent cache lock").get(&key) {
            return parent.clone();
        }
        let variables = serde_json::json!({ "owner": repo.owner, "name": repo.name });
        let data: Data = self.graphql(&repo.host, PARENT, variables).await.ok()?;
        let parent = data.repository.and_then(|r| r.parent).map(|p| p.name_with_owner);
        PARENTS.lock().expect("parent cache lock").insert(key, parent.clone());
        parent
    }
}

fn fork_hint(message: String, repo: &str, parent: Option<&str>) -> String {
    match parent {
        Some(parent) => format!(
            "{message}; {repo} is a fork of {parent}: pass repo={parent} (or the remote that \
             points there) to read it there"
        ),
        None => message,
    }
}
