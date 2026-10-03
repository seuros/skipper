use super::tools::SkipperServer;
use mcp_host::prelude::*;
#[cfg(any(feature = "github", feature = "tea"))]
use serde_json::Value;

impl SkipperServer {
    #[mcp_resource(
        uri = "skipper://workspace",
        name = "workspace",
        description = "Workspace: cwd, repo root, branch, HEAD, dirty, remotes (no credentials) with forge. repo=null outside git",
        mime_type = "application/json"
    )]
    pub(crate) async fn workspace(&self, _ctx: Ctx<'_>) -> ResourceResult {
        use crate::environment::Environment as _;

        let env = self.env.clone();
        let snapshot =
            crate::git::tools::execute_blocking(env.cwd().to_path_buf(), (), move |_, ()| {
                crate::workspace::snapshot(&env).map_err(|e| e.to_string())
            })
            .await
            .map_err(ResourceError::Read)?;

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
        description = "Gitea/Forgejo repo of this workspace's remote",
        mime_type = "application/json",
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:tea\").is_some()).unwrap_or(false)"
    )]
    pub(crate) async fn repo(&self, _ctx: Ctx<'_>) -> ResourceResult {
        use crate::provider::forgejo::{ForgejoClient, credentials_for_host};

        let remotes = crate::remote::ordered_remotes(
            crate::git::remotes(&self.cwd).map_err(|e| ResourceError::Read(e.to_string()))?.remotes,
        );

        let resolved = remotes.iter().find_map(|(remote, url)| {
            let host = crate::remote::host_of(url)?;
            let creds = credentials_for_host(&host)?;
            let (owner, name) = crate::remote::repo_path_of(url)?;
            Some((remote, creds, owner, name))
        });

        let Some((remote, creds, owner, name)) = resolved else {
            return Err(ResourceError::NotFound(
                "no remote of this workspace maps to a Gitea/Forgejo login".to_string(),
            ));
        };

        let repo = ForgejoClient::new(creds)
            .repo(&owner, &name)
            .await
            .map_err(|e| ResourceError::Read(e.to_string()))?;

        let mut payload =
            serde_json::to_value(&repo).map_err(|e| ResourceError::Internal(e.to_string()))?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("remote".to_string(), Value::String(remote.clone()));
        }

        json_resource("skipper://repo", &payload)
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/checks",
        name = "pr_checks",
        title = "PR check matrix",
        description = "GitHub PR checks by workflow: bucket, link, conclusion. number=current for this branch's PR",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_checks(&self, ctx: Ctx<'_>) -> ResourceResult {
        use crate::provider::github::{CheckCounts, GitHubProvider};

        let (number, pr) = pr_param(&ctx)?;

        let checks = GitHubProvider::new()
            .pr_checks(pr)
            .await
            .map_err(|e| ResourceError::Read(e.to_string()))?;

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
            "pr": number,
            "conclusion": counts.conclusion(),
            "counts": counts,
            "workflows": workflows,
        });

        json_resource(format!("skipper://pr/{number}/checks"), &matrix)
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/comments",
        name = "pr_comments",
        title = "PR state and discussion",
        description = "GitHub PR state (open/closed/merged) and every inline review comment, conversation comment and review: id, author, path, line, body without collapsed <details> or HTML comments. number=current for this branch's PR",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_comments(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let discussion = discussion(pr, "all").await?;
        json_resource(format!("skipper://pr/{number}/comments"), &discussion)
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/comments/{kind}",
        name = "pr_comments_kind",
        title = "PR discussion of one kind",
        description = "skipper://pr/{number}/comments keeping only notes of kind: inline | comment | review | all",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_comments_kind(&self, ctx: Ctx<'_>) -> ResourceResult {
        let (number, pr) = pr_param(&ctx)?;
        let kind = note_kind(&ctx)?;
        let discussion = discussion(pr, &kind).await?;
        json_resource(format!("skipper://pr/{number}/comments/{kind}"), &discussion)
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
        for pr in
            view.watching.iter().map(|w| w.pr).filter(|pr| !found.iter().any(|(e, _)| e == pr))
        {
            let kind = kind.clone();
            tasks.spawn(async move { (pr, discussion(Some(pr), &kind).await) });
        }
        while let Some(joined) = tasks.join_next().await {
            let (pr, result) = joined.map_err(|e| ResourceError::Internal(e.to_string()))?;
            let value = match result {
                Ok(d) => {
                    serde_json::to_value(d).map_err(|e| ResourceError::Internal(e.to_string()))?
                }
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
        uri_template = "skipper://prs/{state}/{author}",
        name = "pr_list",
        title = "Recent PRs by state and author",
        description = "Last 30 PRs of this repo in state open | closed | merged | all by author (a login, or me): pr, state, merged_at",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_list(&self, ctx: Ctx<'_>) -> ResourceResult {
        let state = uri_choice(&ctx, "state", "open", &["open", "closed", "merged", "all"])?;
        let author = ctx.get_uri_param("author").unwrap_or_else(|| "me".to_string());
        let prs = crate::provider::github::GitHubProvider::new()
            .pr_list(&state, &author)
            .await
            .map_err(|e| ResourceError::Read(e.to_string()))?;
        json_resource(format!("skipper://prs/{state}/{author}"), &prs)
    }
}

#[cfg(feature = "github")]
async fn discussion(
    pr: Option<u64>,
    kind: &str,
) -> std::result::Result<crate::provider::github::PrDiscussion, ResourceError> {
    let mut discussion = crate::provider::github::GitHubProvider::new()
        .pr_discussion(pr)
        .await
        .map_err(|e| ResourceError::Read(e.to_string()))?;
    if kind != "all" {
        discussion.notes.retain(|n| n.kind == kind);
    }
    Ok(discussion)
}

#[cfg(feature = "github")]
fn note_kind(ctx: &Ctx<'_>) -> std::result::Result<String, ResourceError> {
    uri_choice(ctx, "kind", "all", &["inline", "comment", "review", "all"])
}

/// URI param `name`, `default` when absent; must be one of `allowed`.
#[cfg(feature = "github")]
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
