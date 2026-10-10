use super::tools::SkipperServer;
use mcp_host::prelude::*;

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
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "resource handlers are async; this one reads memory"
    )]
    pub(crate) async fn watch(&self, _ctx: Ctx<'_>) -> ResourceResult {
        let json =
            self.pr_watcher.view_json().map_err(|e| ResourceError::Internal(e.to_string()))?;
        Ok(vec![text_resource_with_mime("skipper://watch", json, "application/json")])
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
        #[derive(serde::Serialize)]
        struct WithRemote<T> {
            remote: String,
            #[serde(flatten)]
            repo: T,
        }

        use crate::provider::forgejo::{ForgejoClient, load_credentials, match_host};

        let (info, current) = crate::git::remotes_with_current(&self.cwd)
            .map_err(|e| ResourceError::Read(e.to_string()))?;
        let remotes =
            crate::remote::ordered_remotes(info.remotes, current.as_ref().map(|c| c.name.as_str()));

        let logins = load_credentials();
        let resolved = remotes.into_iter().find_map(|(remote, url)| {
            let (owner, name) = crate::remote::repo_path_of(&url)?;
            let creds = match_host(&logins, &crate::remote::host_of(&url)?)?.clone();
            Some((remote, creds, owner, name))
        });

        let Some((remote, creds, owner, name)) = resolved else {
            return Err(refused(
                "no remote of this workspace maps to a Gitea/Forgejo login".to_string(),
            ));
        };

        let repo = ForgejoClient::new(creds).repo(&owner, &name).await.map_err(forge_error)?;
        json_resource("skipper://repo", &WithRemote { remote, repo })
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}{?repo}",
        name = "pr",
        title = "PR overview",
        description = "GitHub PR, to decide on a merge: state, draft, author, head, base, head_sha, mergeable, merge_state, review, checks verdict, merge_methods, default_method, labels, size. Description: /comments; files: /files. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (gh, repo, pr) = pr_target(&self.env, spec, pr).await?;
        let overview = gh.pr_overview(&repo, pr).await.map_err(forge_error)?;
        json_resource(
            with_repo(format_args!("skipper://pr/{number}"), spec),
            &Source::repo(repo, overview),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/checks{?repo}",
        name = "pr_checks",
        title = "PR check matrix",
        description = "GitHub PR checks by workflow: name, bucket; link and description on failures. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_checks(&self, ctx: Ctx<'_>) -> ResourceResult {
        #[derive(serde::Serialize)]
        struct Matrix {
            pr: u64,
            conclusion: &'static str,
            counts: CheckCounts,
            workflows: BTreeMap<Cow<'static, str>, Vec<PrCheck>>,
        }

        use crate::provider::github::{CheckCounts, PrCheck};
        use std::borrow::Cow;
        use std::collections::BTreeMap;

        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (gh, repo, pr) = pr_target(&self.env, spec, pr).await?;
        let checks = gh.pr_checks(&repo, pr).await.map_err(forge_error)?;

        let counts = CheckCounts::tally(&checks);
        let mut workflows: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for mut check in checks {
            if !matches!(check.bucket, "fail" | "cancel") {
                check.link = None;
                check.description = None;
            }
            let key = match std::mem::take(&mut check.workflow) {
                workflow if workflow.is_empty() => Cow::Borrowed("(statuses)"),
                workflow => Cow::Owned(workflow),
            };
            workflows.entry(key).or_default().push(check);
        }

        let matrix = Matrix { pr, conclusion: counts.conclusion(), counts, workflows };
        json_resource(
            with_repo(format_args!("skipper://pr/{number}/checks"), spec),
            &Source::repo(repo, matrix),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/files{?repo}",
        name = "pr_files",
        title = "PR changed files",
        description = "GitHub PR changed files: path, additions, deletions, change. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_files(&self, ctx: Ctx<'_>) -> ResourceResult {
        #[derive(serde::Serialize)]
        struct Files {
            pr: u64,
            files: Vec<crate::provider::github::PrFile>,
        }

        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (gh, repo, pr) = pr_target(&self.env, spec, pr).await?;
        let files = gh.pr_files(&repo, pr).await.map_err(forge_error)?;
        json_resource(
            with_repo(format_args!("skipper://pr/{number}/files"), spec),
            &Source::repo(repo, Files { pr, files }),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/comments{?repo}",
        name = "pr_comments",
        title = "PR state and discussion",
        description = "GitHub PR state open|closed|merged + notes description|inline|comment|review: id, kind, author, path, line, body. number=current: this branch's PR. repo=remote|owner/name (default: current remote)",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_comments(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (repo, discussion) = discussion(&self.env, spec, pr, "all").await?;
        json_resource(
            with_repo(format_args!("skipper://pr/{number}/comments"), spec),
            &Source::repo(repo, discussion),
        )
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/comments/{kind}{?repo}",
        name = "pr_comments_kind",
        title = "PR discussion of one kind",
        description = "skipper://pr/{number}/comments keeping only notes of kind: description | inline | comment | review | all",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_comments_kind(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let kind = note_kind(&ctx)?;
        let spec = ctx.get_uri_param("repo");
        let (repo, discussion) = discussion(&self.env, spec, pr, kind).await?;
        json_resource(
            with_repo(format_args!("skipper://pr/{number}/comments/{kind}"), spec),
            &Source::repo(repo, discussion),
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
        /// One PR's entry: how a finished watch ended, its discussion, or why
        /// that could not be read.
        #[derive(serde::Serialize)]
        #[serde(untagged)]
        enum Entry {
            Ended { pr: u64, state: &'static str },
            Read(Source<crate::provider::github::PrDiscussion>),
            Failed { pr: u64, error: String },
        }

        let kind = note_kind(&ctx)?;
        let targets = self.pr_watcher.discussion_targets();
        let mut found: Vec<(u64, Entry)> =
            targets.ended.iter().map(|&(pr, state)| (pr, Entry::Ended { pr, state })).collect();

        let mut tasks = tokio::task::JoinSet::new();
        for (pr, repo) in targets.watching {
            if targets.ended.iter().any(|(ended, _)| *ended == pr) {
                continue;
            }
            let env = self.env.clone();
            tasks.spawn(async move { (pr, discussion(&env, Some(&repo), Some(pr), kind).await) });
        }
        while let Some(joined) = tasks.join_next().await {
            let (pr, result) = joined.map_err(|e| ResourceError::Internal(e.to_string()))?;
            let entry = match result {
                Ok((repo, d)) => Entry::Read(Source::repo(repo, d)),
                Err(e) => Entry::Failed { pr, error: e.to_string() },
            };
            found.push((pr, entry));
        }
        found.sort_by_key(|(pr, _)| *pr);
        found.dedup_by_key(|(pr, _)| *pr);
        let entries: Vec<Entry> = found.into_iter().map(|(_, entry)| entry).collect();
        json_resource(format!("skipper://watch/comments/{kind}"), &entries)
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
        #[derive(serde::Serialize)]
        struct Prs {
            prs: Vec<crate::provider::github::PrState>,
        }

        let state = uri_choice(&ctx, "state", "open", &["open", "closed", "merged", "all"])?;
        let author = ctx.get_uri_param("author").unwrap_or("me");
        let spec = ctx.get_uri_param("repo");
        let repo = github_repo(&self.env, spec)?;
        let prs = crate::provider::github::GitHubProvider::new()
            .pr_list(&repo, state, author)
            .await
            .map_err(forge_error)?;
        json_resource(
            with_repo(format_args!("skipper://prs/{state}/{author}"), spec),
            &Source::repo(repo, Prs { prs }),
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
        #[derive(serde::Serialize)]
        struct Issues {
            issues: Vec<crate::provider::issues::IssueSummary>,
        }

        let state = uri_choice(&ctx, "state", "open", &["open", "closed", "all"])?;
        let spec = ctx.get_uri_param("repo");
        let target = issue_repo(&self.env, spec)?;
        let issues = match target.forge {
            #[cfg(feature = "github")]
            "github" => {
                crate::provider::github::GitHubProvider::new()
                    .issues(&target.host, &target.owner, &target.name, state)
                    .await
            }
            #[cfg(feature = "tea")]
            "tea" => forgejo_client(&target)?.issues(&target.owner, &target.name, state).await,
            _ => return Err(no_issue_forge(&target)),
        }
        .map_err(forge_error)?;

        json_resource(
            with_repo(format_args!("skipper://issues/{state}"), spec),
            &Source::forge(target, Issues { issues }),
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
        let target = issue_repo(&self.env, spec)?;
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
            with_repo(format_args!("skipper://issue/{number}"), spec),
            &Source::forge(target, thread),
        )
    }
}

/// Forge data tagged with the repo that answered, so an empty list is never
/// read as another repo's answer. `forge` only where reads span forges.
#[cfg(any(feature = "github", feature = "tea"))]
#[derive(serde::Serialize)]
struct Source<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    forge: Option<&'static str>,
    /// `owner/name`
    #[serde(serialize_with = "full_name")]
    repo: crate::workspace::ForgeRepo,
    #[serde(flatten)]
    data: T,
}

/// Writes `owner/name` straight into the output.
#[cfg(any(feature = "github", feature = "tea"))]
fn full_name<S: serde::Serializer>(
    repo: &crate::workspace::ForgeRepo,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    s.collect_str(&repo.full_name())
}

#[cfg(any(feature = "github", feature = "tea"))]
impl<T> Source<T> {
    /// A read on one forge (GitHub PRs): the repo alone.
    #[cfg(feature = "github")]
    const fn repo(target: crate::workspace::ForgeRepo, data: T) -> Self {
        Self { forge: None, repo: target, data }
    }

    /// A read any forge may answer (issues): the forge too.
    const fn forge(target: crate::workspace::ForgeRepo, data: T) -> Self {
        Self { forge: Some(target.forge), repo: target, data }
    }
}

/// `uri` with the `repo` it was read with, formatted once.
#[cfg(any(feature = "github", feature = "tea"))]
fn with_repo(uri: std::fmt::Arguments<'_>, spec: Option<&str>) -> String {
    match spec {
        Some(spec) => format!("{uri}?repo={}", urlencoding::Encoded(spec)),
        None => uri.to_string(),
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
#[expect(
    clippy::needless_pass_by_value,
    reason = "a `map_err` adapter takes the error it replaces"
)]
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
fn note_kind(ctx: &Ctx<'_>) -> std::result::Result<&'static str, ResourceError> {
    uri_choice(ctx, "kind", "all", &["description", "inline", "comment", "review", "all"])
}

/// URI param `name`, `default` when absent; must be one of `allowed`.
#[cfg(any(feature = "github", feature = "tea"))]
fn uri_choice(
    ctx: &Ctx<'_>,
    name: &str,
    default: &'static str,
    allowed: &[&'static str],
) -> std::result::Result<&'static str, ResourceError> {
    let value = ctx.get_uri_param(name).unwrap_or(default);
    allowed.iter().copied().find(|choice| *choice == value).ok_or_else(|| {
        ResourceError::InvalidUri(format!("{name} must be {}: {value}", allowed.join(" | ")))
    })
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
fn pr_param<'a>(ctx: &'a Ctx<'_>) -> std::result::Result<(&'a str, Option<u64>), ResourceError> {
    let number = ctx.get_uri_param("number").unwrap_or("current");
    let pr = match number {
        "current" => None,
        s => Some(s.parse::<u64>().map_err(|_| {
            ResourceError::InvalidUri(format!("PR number must be an integer or `current`: {s}"))
        })?),
    };
    Ok((number, pr))
}
