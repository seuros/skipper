//! Network git (fetch, fast-forward pull, push) through the `git` binary:
//! gitoxide cannot push, and credentials (ssh agent, credential helpers) are
//! git's own. Nothing here forces or deletes; prompts are off, so missing
//! credentials fail instead of hanging.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;

use crate::git::{CurrentRemote, GitError};

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
        .filter(|current| current.source == crate::git::RemoteSource::Tracked)
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
        let branch = crate::git::current_branch(cwd)?
            .ok_or_else(|| GitError::InvalidInput("detached HEAD; pass refs".into()))?;
        refspecs.push(branch);
    }
    for tag in tags {
        refspecs.push(format!("refs/tags/{}", checked_ref(tag)?));
    }

    // One read of the remotes resolves every target.
    let (info, current) = crate::git::remotes_with_current(cwd)?;
    let configured = info.remotes;
    let remotes = match targets {
        [] => resolve(&configured, current, None)?,
        [all] if all == "all" => resolve(&configured, current, Some("all"))?,
        named => {
            let mut resolved = Vec::new();
            for name in named {
                resolved.extend(resolve(&configured, None, Some(name))?);
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

/// Remote names for `spec`, from one read of the remotes.
fn remotes(cwd: &Path, spec: Option<&str>) -> Result<Vec<String>, GitError> {
    let (info, current) = crate::git::remotes_with_current(cwd)?;
    resolve(&info.remotes, current, spec)
}

/// Remote names among `configured`: `current`, every remote (`all`), or one
/// by name.
fn resolve(
    configured: &BTreeMap<String, String>,
    current: Option<CurrentRemote>,
    spec: Option<&str>,
) -> Result<Vec<String>, GitError> {
    let names = || {
        fmt::from_fn(|f| {
            for (i, name) in configured.keys().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                f.write_str(name)?;
            }
            Ok(())
        })
    };
    match spec {
        None => current.map(|r| vec![r.name]).ok_or_else(|| {
            GitError::InvalidInput(format!("no current remote among {}; name one", names()))
        }),
        Some("all") if !configured.is_empty() => Ok(configured.keys().cloned().collect()),
        Some(name) if configured.contains_key(name) => Ok(vec![name.to_owned()]),
        Some(name) => {
            Err(GitError::InvalidInput(format!("no remote {name:?}; remotes: {}", names())))
        }
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

/// Git's words for a network that dropped, not a request that failed:
/// worth another try. A rejected push or a refused login is not.
const TRANSIENT: &[&str] = &[
    "connection reset",
    "connection refused",
    "connection timed out",
    "operation timed out",
    "could not resolve host",
    "temporary failure in name resolution",
    "failed to connect",
    "couldn't connect to server",
    "the remote end hung up unexpectedly",
    "connection closed by remote host",
    "kex_exchange_identification",
    "early eof",
    "rpc failed",
    "ssl_error_syscall",
    "gnutls_handshake",
    "the requested url returned error: 50",
];

pub(crate) fn transient(output: &str) -> bool {
    let output = output.to_lowercase();
    TRANSIENT.iter().any(|marker| output.contains(marker))
}

enum Attempt {
    Transient(SyncOutcome),
    Failed(GitError),
}

/// `git args`, tried up to three times (~1s doubling, jittered) while it
/// fails on the network. Fetch, fast-forward pull and push of the same refs
/// are all safe to repeat. A timeout is not retried: its wait is long already.
async fn run(cwd: &Path, remote: &str, args: &[&str]) -> Result<SyncOutcome, GitError> {
    use chrono_machines::{AsyncRetryable, RetryOutcome};

    let backoff = crate::provider::network_backoff(3);
    let attempt = || async {
        match run_once(cwd, remote, args).await {
            Ok(outcome) if !outcome.ok && transient(&outcome.output) => {
                Err(Attempt::Transient(outcome))
            }
            Ok(outcome) => Ok(outcome),
            Err(e) => Err(Attempt::Failed(e)),
        }
    };
    let result = attempt
        .retry_async(backoff)
        .when(|a: &Attempt| matches!(a, Attempt::Transient(_)))
        .call_async(|ms| tokio::time::sleep(Duration::from_millis(ms)))
        .await
        .map(RetryOutcome::into_inner)
        .map_err(|e| e.into_cause().expect("a failed retry carries its last error"));
    match result {
        Ok(outcome) | Err(Attempt::Transient(outcome)) => Ok(outcome),
        Err(Attempt::Failed(e)) => Err(e),
    }
}

async fn run_once(cwd: &Path, remote: &str, args: &[&str]) -> Result<SyncOutcome, GitError> {
    let envs = [("GIT_TERMINAL_PROMPT", "0")];
    let output = crate::executor::execute_in("git", args, cwd, &envs, TIMEOUT)
        .await
        .map_err(|e| GitError::Operation(e.to_string()))?;
    let (stdout, stderr) = (output.stdout.trim(), output.stderr.trim());
    let text = match (stdout.is_empty(), stderr.is_empty()) {
        (_, true) => stdout.to_owned(),
        (true, false) => stderr.to_owned(),
        (false, false) => format!("{stdout}\n{stderr}"),
    };
    Ok(SyncOutcome { remote: remote.to_owned(), ok: output.success(), output: text })
}

#[cfg(test)]
mod tests;
