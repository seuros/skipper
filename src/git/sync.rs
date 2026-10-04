//! Network git (fetch, fast-forward pull, push) through the `git` binary:
//! gitoxide cannot push, and credentials (ssh agent, credential helpers) are
//! git's own. Nothing here forces or deletes; prompts are off, so missing
//! credentials fail instead of hanging.

use std::path::Path;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;

use crate::git::GitError;

const TIMEOUT: Duration = Duration::from_secs(120);

/// One remote's result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SyncOutcome {
    pub remote: String,
    pub ok: bool,
    /// What git reported.
    pub output: String,
}

/// Fetch `remote` (default: the current remote; `all`: every remote), pruning
/// branches deleted upstream.
pub async fn fetch(cwd: &Path, remote: Option<&str>) -> Result<Vec<SyncOutcome>, GitError> {
    let mut outcomes = Vec::new();
    for remote in remotes(cwd, remote)? {
        outcomes.push(run(cwd, &remote, &["fetch", "--prune", "--", &remote]).await?);
    }
    Ok(outcomes)
}

/// Fast-forward the checked-out branch to its upstream; refuses to merge.
pub async fn pull(cwd: &Path) -> Result<SyncOutcome, GitError> {
    let upstream = crate::git::current_remote(cwd)?
        .filter(|current| current.source == crate::git::RemoteSource::Upstream)
        .ok_or_else(|| {
            GitError::InvalidInput(
                "branch has no upstream; set one with `git branch -u <remote>/<branch>`".into(),
            )
        })?;
    run(cwd, &upstream.name, &["pull", "--ff-only"]).await
}

/// What a push will send where, settled before anyone is asked to approve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPlan {
    pub remotes: Vec<String>,
    pub refspecs: Vec<String>,
}

impl PushPlan {
    pub fn summary(&self) -> String {
        format!("push {} to {}", self.refspecs.join(", "), self.remotes.join(", "))
    }
}

/// `refs` (default: the checked-out branch) and `tags` to each remote in
/// `targets` (default: the current remote; `all`: every remote).
pub fn plan_push(
    cwd: &Path,
    targets: &[String],
    refs: &[String],
    tags: &[String],
) -> Result<PushPlan, GitError> {
    let mut refspecs = refs.iter().map(|r| checked_ref(r)).collect::<Result<Vec<_>, _>>()?;
    if refspecs.is_empty() && tags.is_empty() {
        let branch = crate::git::repo_info(cwd)?
            .branch
            .ok_or_else(|| GitError::InvalidInput("detached HEAD; pass refs".into()))?;
        refspecs.push(branch);
    }
    for tag in tags {
        refspecs.push(format!("refs/tags/{}", checked_ref(tag)?));
    }

    let remotes = match targets {
        [] => remotes(cwd, None)?,
        [all] if all == "all" => remotes(cwd, Some("all"))?,
        named => {
            let mut resolved = Vec::new();
            for name in named {
                resolved.extend(remotes(cwd, Some(name))?);
            }
            resolved
        }
    };

    Ok(PushPlan { remotes, refspecs })
}

/// Push as planned, one remote after the other; a failed remote does not stop
/// the rest.
pub async fn push(cwd: &Path, plan: &PushPlan) -> Result<Vec<SyncOutcome>, GitError> {
    let mut outcomes = Vec::new();
    for remote in &plan.remotes {
        let mut args = vec!["push", "--", remote.as_str()];
        args.extend(plan.refspecs.iter().map(String::as_str));
        outcomes.push(run(cwd, remote, &args).await?);
    }
    Ok(outcomes)
}

/// Remote names: the current remote, every remote (`all`), or one by name.
fn remotes(cwd: &Path, spec: Option<&str>) -> Result<Vec<String>, GitError> {
    let configured: Vec<String> = crate::git::remotes(cwd)?.remotes.into_keys().collect();
    match spec {
        None => crate::git::current_remote(cwd)?.map(|r| vec![r.name]).ok_or_else(|| {
            GitError::InvalidInput(format!(
                "no current remote among {}; name one",
                configured.join(", ")
            ))
        }),
        Some("all") if !configured.is_empty() => Ok(configured),
        Some(name) if configured.iter().any(|c| c == name) => Ok(vec![name.to_string()]),
        Some(name) => Err(GitError::InvalidInput(format!(
            "no remote {name:?}; remotes: {}",
            configured.join(", ")
        ))),
    }
}

/// A branch or tag name, refused when it could force (`+x`), delete or
/// retarget (`a:b`), or read as an option (`-x`).
pub(crate) fn checked_ref(name: &str) -> Result<String, GitError> {
    let unsafe_char = |c: char| c == ':' || c.is_whitespace() || c.is_control();
    if name.is_empty() || name.starts_with(['+', '-']) || name.contains(unsafe_char) {
        return Err(GitError::InvalidInput(format!(
            "{name:?} is not a plain branch or tag name; skipper never forces or deletes"
        )));
    }
    Ok(name.to_string())
}

async fn run(cwd: &Path, remote: &str, args: &[&str]) -> Result<SyncOutcome, GitError> {
    let envs = [("GIT_TERMINAL_PROMPT", "0")];
    let output = crate::executor::execute_in("git", args, cwd, &envs, TIMEOUT)
        .await
        .map_err(|e| GitError::Operation(e.to_string()))?;
    let text = format!("{}\n{}", output.stdout.trim(), output.stderr.trim());
    Ok(SyncOutcome {
        remote: remote.to_string(),
        ok: output.success(),
        output: text.trim().to_string(),
    })
}

#[cfg(test)]
mod tests;
