use super::tools::SkipperServer;
use mcp_host::prelude::*;
#[cfg(any(feature = "github", feature = "tea"))]
use serde_json::Value;

impl SkipperServer {
    #[mcp_resource(
        uri = "skipper://workspace",
        name = "workspace",
        description = "Workspace: cwd, repo root, branch, HEAD, dirty, remotes (no credentials) with forge; current=tracked|only|origin marks the remote forge reads start from (a GitHub fork's PRs are read on its parent). repo=null outside git",
        mime_type = "application/json"
    )]
    pub(crate) async fn workspace(&self, _ctx: Ctx<'_>) -> ResourceResult {
        use crate::environment::Environment as _;

        let env = self.env.clone();
        let snapshot =
            crate::git::tools::execute_blocking(env.cwd().to_path_buf(), (), move |_, ()| {
                Ok(crate::workspace::snapshot(&env)?)
            })
            .await
            .map_err(|e| ResourceError::Read(e.to_string()))?;

        json_resource("skipper://workspace", &snapshot)
    }

    #[cfg(feature = "github")]
    #[mcp_resource(
        uri = "skipper://watch",
        name = "watch",
        description = "Watched PRs: status, checks, recent events, undelivered events, whether a pr_watch call blocks. Read-only; does not consume events",
        mime_type = "application/json",
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:github\").is_some()).unwrap_or(false)"
    )]
    pub(crate) async fn watch(&self, _ctx: Ctx<'_>) -> ResourceResult {
        json_resource("skipper://watch", &self.pr_watcher.view())
    }

    #[cfg(feature = "tea")]
    #[mcp_resource(
        uri = "skipper://repo",
        name = "repo",
        description = "Gitea/Forgejo repo of this workspace's remote, current remote first",
        mime_type = "application/json",
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:tea\").is_some()).unwrap_or(false)"
    )]
    pub(crate) async fn repo(&self, _ctx: Ctx<'_>) -> ResourceResult {
        use crate::provider::forgejo::{ForgejoClient, credentials_for_host};

        let current = crate::git::current_remote(&self.cwd).ok().flatten().map(|c| c.name);
        let remotes = crate::remote::ordered_remotes(
            crate::git::remotes(&self.cwd).map_err(|e| ResourceError::Read(e.to_string()))?.remotes,
            current.as_deref(),
        );

        let resolved = remotes.iter().find_map(|(remote, url)| {
            let host = crate::remote::host_of(url)?;
            let creds = credentials_for_host(&host)?;
            let (owner, name) = crate::remote::repo_path_of(url)?;
            Some((remote, creds, owner, name))
        });

        let Some((remote, creds, owner, name)) = resolved else {
            return Err(refused(
                "no remote of this workspace maps to a Gitea/Forgejo login".to_string(),
            ));
        };

        let repo = ForgejoClient::new(creds).repo(&owner, &name).await.map_err(forge_error)?;

        let mut payload =
            serde_json::to_value(&repo).map_err(|e| ResourceError::Internal(e.to_string()))?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("remote".to_string(), Value::String(remote.clone()));
        }

        json_resource("skipper://repo", &payload)
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}{?repo}",
        name = "pr",
        title = "PR overview",
        description = "GitHub PR: title, state, draft, author, head, base, head_sha, mergeable, merge_state, review, merge_methods, default_method, labels, files, body. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (gh, repo, pr) = pr_target(&self.env, spec.as_deref(), pr).await?;
        let overview = gh.pr_overview(&repo, pr).await.map_err(forge_error)?;
        json_resource(
            with_repo(format!("skipper://pr/{number}"), spec.as_deref()),
            &FromRemote::new(&repo, overview),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/checks{?repo}",
        name = "pr_checks",
        title = "PR check matrix",
        description = "GitHub PR checks by workflow: bucket, link, conclusion. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_checks(&self, ctx: Ctx<'_>) -> ResourceResult {
        use crate::provider::github::CheckCounts;

        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (gh, repo, pr) = pr_target(&self.env, spec.as_deref(), pr).await?;
        let checks = gh.pr_checks(&repo, pr).await.map_err(forge_error)?;

        let counts = CheckCounts::tally(&checks);
        let mut workflows = serde_json::Map::new();
        for mut check in checks {
            let key = match std::mem::take(&mut check.workflow) {
                workflow if workflow.is_empty() => "(statuses)".to_string(),
                workflow => workflow,
            };
            let check =
                serde_json::to_value(check).map_err(|e| ResourceError::Internal(e.to_string()))?;
            workflows
                .entry(key)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("workflow entries are arrays")
                .push(check);
        }

        let matrix = serde_json::json!({
            "pr": pr,
            "conclusion": counts.conclusion(),
            "counts": counts,
            "workflows": workflows,
        });
        json_resource(
            with_repo(format!("skipper://pr/{number}/checks"), spec.as_deref()),
            &FromRemote::new(&repo, matrix),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/comments{?repo}",
        name = "pr_comments",
        title = "PR state and discussion",
        description = "GitHub PR state open|closed|merged + notes inline|comment|review: id, kind, author, path, line, body. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_comments(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (repo, discussion) = discussion(&self.env, spec.as_deref(), pr, "all").await?;
        json_resource(
            with_repo(format!("skipper://pr/{number}/comments"), spec.as_deref()),
            &FromRemote::new(&repo, discussion),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/comments/{kind}{?repo}",
        name = "pr_comments_kind",
        title = "PR discussion of one kind",
        description = "skipper://pr/{number}/comments keeping only notes of kind: inline | comment | review | all",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_comments_kind(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let kind = note_kind(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (repo, discussion) = discussion(&self.env, spec.as_deref(), pr, &kind).await?;
        json_resource(
            with_repo(format!("skipper://pr/{number}/comments/{kind}"), spec.as_deref()),
            &FromRemote::new(&repo, discussion),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://watch/comments/{kind}",
        name = "watch_comments",
        title = "Discussion of every watched PR",
        description = "skipper://pr/{number}/comments/{kind} for every PR under pr_watch or recently finished, in one read",
        mime_type = "application/json",
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:github\").is_some()).unwrap_or(false)"
    )]
    pub(crate) async fn watch_comments(&self, ctx: Ctx<'_>) -> ResourceResult {
        let kind = note_kind(&ctx)?;
        let view = self.pr_watcher.view();
        let mut found: Vec<(u64, Value)> = view
            .ended
            .iter()
            .map(|s| (s.pr, serde_json::json!({ "pr": s.pr, "state": s.state })))
            .collect();

        let mut tasks = tokio::task::JoinSet::new();
        for watched in view.watching.iter().filter(|w| !found.iter().any(|(e, _)| *e == w.pr)) {
            let (pr, repo) = (watched.pr, watched.repo.clone());
            let kind = kind.clone();
            let env = self.env.clone();
            tasks.spawn(async move { (pr, discussion(&env, Some(&repo), Some(pr), &kind).await) });
        }
        while let Some(joined) = tasks.join_next().await {
            let (pr, result) = joined.map_err(|e| ResourceError::Internal(e.to_string()))?;
            let value = match result {
                Ok((repo, d)) => serde_json::to_value(FromRemote::new(&repo, d))
                    .map_err(|e| ResourceError::Internal(e.to_string()))?,
                Err(e) => serde_json::json!({ "pr": pr, "error": e.to_string() }),
            };
            found.push((pr, value));
        }
        found.sort_by_key(|(pr, _)| *pr);
        found.dedup_by_key(|(pr, _)| *pr);
        let values: Vec<Value> = found.into_iter().map(|(_, v)| v).collect();
        json_resource(format!("skipper://watch/comments/{kind}"), &values)
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://prs/{state}/{author}{?repo}",
        name = "pr_list",
        title = "Recent PRs by state and author",
        description = "Last 30 GitHub PRs, newest first, state=open|closed|merged|all, author=login|me: remote, repo, prs[pr, title, head, state, merged_at]. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_list(&self, ctx: Ctx<'_>) -> ResourceResult {
        let state = uri_choice(&ctx, "state", "open", &["open", "closed", "merged", "all"])?;
        let author = ctx.get_uri_param("author").unwrap_or_else(|| "me".to_string());
        let spec = ctx.get_uri_param("repo");
        let repo = github_repo(&self.env, spec.as_deref())?;
        let prs = crate::provider::github::GitHubProvider::new()
            .pr_list(&repo, &state, &author)
            .await
            .map_err(forge_error)?;
        json_resource(
            with_repo(format!("skipper://prs/{state}/{author}"), spec.as_deref()),
            &FromRemote::new(&repo, serde_json::json!({ "prs": prs })),
        )
    }

    #[cfg(any(feature = "github", feature = "tea"))]
    #[mcp_resource_template(
        uri_template = "skipper://issues/{state}{?repo}",
        name = "issues",
        title = "Recent issues",
        description = "Issues (GitHub, Gitea/Forgejo), last 30 by update. state=open|closed|all: number, title, state, author, labels, comments, created_at, updated_at. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json",
        visible = "ctx.environment.map(|e| e.has_git_repo() && (e.get_custom(\"forge:github\").is_some() || e.get_custom(\"forge:tea\").is_some())).unwrap_or(false)"
    )]
    pub(crate) async fn issues(&self, ctx: Ctx<'_>) -> ResourceResult {
        let state = uri_choice(&ctx, "state", "open", &["open", "closed", "all"])?;
        let spec = ctx.get_uri_param("repo");
        let target = issue_repo(&self.env, spec.as_deref())?;
        let issues = match target.forge {
            #[cfg(feature = "github")]
            "github" => {
                crate::provider::github::GitHubProvider::new()
                    .issues(&target.host, &target.owner, &target.name, &state)
                    .await
            }
            #[cfg(feature = "tea")]
            "tea" => forgejo_client(&target)?.issues(&target.owner, &target.name, &state).await,
            _ => return Err(no_issue_forge(&target)),
        }
        .map_err(forge_error)?;

        json_resource(
            with_repo(format!("skipper://issues/{state}"), spec.as_deref()),
            &FromRemote::new(&target, serde_json::json!({ "issues": issues })),
        )
    }

    #[cfg(any(feature = "github", feature = "tea"))]
    #[mcp_resource_template(
        uri_template = "skipper://issue/{number}{?repo}",
        name = "issue",
        title = "Issue and discussion",
        description = "Issue (GitHub, Gitea/Forgejo): title, state, author, created_at, labels, assignees, body, comments. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json",
        visible = "ctx.environment.map(|e| e.has_git_repo() && (e.get_custom(\"forge:github\").is_some() || e.get_custom(\"forge:tea\").is_some())).unwrap_or(false)"
    )]
    pub(crate) async fn issue(&self, ctx: Ctx<'_>) -> ResourceResult {
        let raw = ctx.get_uri_param("number").unwrap_or_default();
        let number = raw.parse::<u64>().map_err(|_| {
            ResourceError::InvalidUri(format!("issue number must be an integer: {raw}"))
        })?;
        let spec = ctx.get_uri_param("repo");
        let target = issue_repo(&self.env, spec.as_deref())?;
        let thread = match target.forge {
            #[cfg(feature = "github")]
            "github" => {
                crate::provider::github::GitHubProvider::new()
                    .issue(&target.host, &target.owner, &target.name, number)
                    .await
            }
            #[cfg(feature = "tea")]
            "tea" => forgejo_client(&target)?.issue(&target.owner, &target.name, number).await,
            _ => return Err(no_issue_forge(&target)),
        }
        .map_err(forge_error)?;

        let Some(thread) = thread else {
            let hint = if target.forge == "github" {
                format!("; read skipper://pr/{number}/comments")
            } else {
                String::new()
            };
            return Err(refused(format!("#{number} is a pull request{hint}")));
        };
        json_resource(
            with_repo(format!("skipper://issue/{number}"), spec.as_deref()),
            &FromRemote::new(&target, thread),
        )
    }
}

/// Forge data tagged with where it came from: which repo answered, and through
/// which remote (absent for a `repo=owner/name` read).
#[cfg(any(feature = "github", feature = "tea"))]
#[derive(serde::Serialize)]
struct FromRemote<'a, T> {
    #[serde(skip_serializing_if = "str::is_empty")]
    remote: &'a str,
    forge: &'static str,
    repo: String,
    #[serde(flatten)]
    data: T,
}

#[cfg(any(feature = "github", feature = "tea"))]
impl<'a, T> FromRemote<'a, T> {
    fn new(target: &'a crate::workspace::ForgeRepo, data: T) -> Self {
        Self { remote: &target.remote, forge: target.forge, repo: target.full_name(), data }
    }
}

/// `uri` with the `repo` it was read with.
#[cfg(any(feature = "github", feature = "tea"))]
fn with_repo(uri: String, spec: Option<&str>) -> String {
    match spec {
        Some(spec) => format!("{uri}?repo={}", urlencoding::encode(spec)),
        None => uri,
    }
}

/// The repo an issue read goes to: `spec`, else the current remote.
#[cfg(any(feature = "github", feature = "tea"))]
fn issue_repo(
    env: &crate::environment::SkipperEnvironment,
    spec: Option<&str>,
) -> std::result::Result<crate::workspace::ForgeRepo, ResourceError> {
    crate::workspace::select_any(env, spec).map_err(remote_error)
}

#[cfg(any(feature = "github", feature = "tea"))]
fn remote_error(e: crate::workspace::RemoteError) -> ResourceError {
    match e {
        crate::workspace::RemoteError::Git(e) => ResourceError::Read(e.to_string()),
        e => refused(e.to_string()),
    }
}

#[cfg(any(feature = "github", feature = "tea"))]
fn no_issue_forge(target: &crate::workspace::ForgeRepo) -> ResourceError {
    refused(format!(
        "{} is on {}; issues are read from GitHub or Gitea/Forgejo",
        target.full_name(),
        target.forge
    ))
}

#[cfg(feature = "tea")]
fn forgejo_client(
    target: &crate::workspace::ForgeRepo,
) -> std::result::Result<crate::provider::forgejo::ForgejoClient, ResourceError> {
    let creds = crate::provider::forgejo::credentials_for_host(&target.host).ok_or_else(|| {
        refused(format!("no tea login for {}; add one with `tea login add`", target.host))
    })?;
    Ok(crate::provider::forgejo::ForgejoClient::new(creds))
}

#[cfg(feature = "github")]
async fn discussion(
    env: &crate::environment::SkipperEnvironment,
    spec: Option<&str>,
    pr: Option<u64>,
    kind: &str,
) -> std::result::Result<
    (crate::workspace::ForgeRepo, crate::provider::github::PrDiscussion),
    ResourceError,
> {
    let (gh, repo, pr) = pr_target(env, spec, pr).await?;
    let mut discussion = gh.pr_discussion(&repo, pr).await.map_err(forge_error)?;
    if kind != "all" {
        discussion.notes.retain(|n| n.kind == kind);
    }
    Ok((repo, discussion))
}

/// See [`GitHubProvider::locate_pr`](crate::provider::github::GitHubProvider::locate_pr).
#[cfg(feature = "github")]
async fn pr_target(
    env: &crate::environment::SkipperEnvironment,
    spec: Option<&str>,
    pr: Option<u64>,
) -> std::result::Result<
    (crate::provider::github::GitHubProvider, crate::workspace::ForgeRepo, u64),
    ResourceError,
> {
    let gh = crate::provider::github::GitHubProvider::new();
    let (repo, pr) = gh.locate_pr(env, spec, pr).await.map_err(forge_error)?;
    Ok((gh, repo, pr))
}

/// The GitHub repo a read goes to: `spec` (a remote name or `owner/name`),
/// else the current remote when on GitHub, else the one GitHub repo among the
/// remotes.
#[cfg(feature = "github")]
fn github_repo(
    env: &crate::environment::SkipperEnvironment,
    spec: Option<&str>,
) -> std::result::Result<crate::workspace::ForgeRepo, ResourceError> {
    crate::workspace::select_on(env, "github", spec).map_err(remote_error)
}

/// A forge failure. Network and 5xx failures already went through skipper's
/// retries, so they answer `RetryExhausted`, which mcp-host does not retry
/// again; anything else (no such PR, no login, a 404) is refused at once.
#[cfg(any(feature = "github", feature = "tea"))]
fn forge_error(e: crate::error::CliError) -> ResourceError {
    if crate::provider::retryable(&e) {
        return ResourceError::RetryExhausted {
            attempts: crate::provider::RETRY_ATTEMPTS,
            message: e.to_string(),
        };
    }
    refused(e.to_string())
}

#[cfg(feature = "github")]
fn note_kind(ctx: &Ctx<'_>) -> std::result::Result<String, ResourceError> {
    uri_choice(ctx, "kind", "all", &["inline", "comment", "review", "all"])
}

/// URI param `name`, `default` when absent; must be one of `allowed`.
#[cfg(any(feature = "github", feature = "tea"))]
fn uri_choice(
    ctx: &Ctx<'_>,
    name: &str,
    default: &str,
    allowed: &[&str],
) -> std::result::Result<String, ResourceError> {
    let value = ctx.get_uri_param(name).unwrap_or_else(|| default.to_string());
    if allowed.contains(&value.as_str()) {
        return Ok(value);
    }
    Err(ResourceError::InvalidUri(format!("{name} must be {}: {value}", allowed.join(" | "))))
}

/// A read this workspace cannot serve, with its reason. mcp-host answers
/// `NotFound` with the URI alone and retries `Read` as if transient;
/// `InvalidUri` (-32602) keeps the message and fails at once.
#[cfg(any(feature = "github", feature = "tea"))]
fn refused(message: impl Into<String>) -> ResourceError {
    ResourceError::InvalidUri(message.into())
}

fn json_resource(uri: impl Into<String>, value: &impl serde::Serialize) -> ResourceResult {
    let json = serde_json::to_string(value).map_err(|e| ResourceError::Internal(e.to_string()))?;
    Ok(vec![text_resource_with_mime(uri, json, "application/json")])
}

#[cfg(feature = "github")]
fn pr_param(ctx: &Ctx<'_>) -> std::result::Result<(String, Option<u64>), ResourceError> {
    let number = ctx.get_uri_param("number").unwrap_or_else(|| "current".to_string());
    let pr = match number.as_str() {
        "current" => None,
        s => Some(s.parse::<u64>().map_err(|_| {
            ResourceError::InvalidUri(format!("PR number must be an integer or `current`: {s}"))
        })?),
    };
    Ok((number, pr))
}
