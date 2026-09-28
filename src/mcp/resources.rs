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

        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| ResourceError::Internal(e.to_string()))?;

        Ok(vec![text_resource_with_mime("skipper://workspace", json, "application/json")])
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
        let json = serde_json::to_string_pretty(&self.pr_watcher.view())
            .map_err(|e| ResourceError::Internal(e.to_string()))?;
        Ok(vec![text_resource_with_mime("skipper://watch", json, "application/json")])
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

        let json = serde_json::to_string_pretty(&payload)
            .map_err(|e| ResourceError::Internal(e.to_string()))?;

        Ok(vec![text_resource_with_mime("skipper://repo", json, "application/json")])
    }

    #[cfg(feature = "github")]
    #[mcp_resource_template(
        uri_template = "skipper://pr/{number}/checks",
        name = "pr_checks",
        title = "PR check matrix",
        description = "GitHub PR checks by workflow: bucket, timing, links, conclusion. number=current for this branch's PR",
        mime_type = "application/json"
    )]
    pub(crate) async fn pr_checks(&self, ctx: Ctx<'_>) -> ResourceResult {
        use crate::provider::github::{CheckCounts, GitHubProvider};

        let number = ctx.get_uri_param("number").unwrap_or_else(|| "current".to_string());
        let pr = match number.as_str() {
            "current" => None,
            s => Some(s.parse::<u64>().map_err(|_| {
                ResourceError::InvalidUri(format!("PR number must be an integer or `current`: {s}"))
            })?),
        };

        let checks = GitHubProvider::new()
            .pr_checks(pr)
            .await
            .map_err(|e| ResourceError::Read(e.to_string()))?;

        let counts = CheckCounts::tally(&checks);
        let mut workflows = serde_json::Map::new();
        for check in &checks {
            let key = if check.workflow.is_empty() { "(statuses)" } else { &check.workflow };
            workflows
                .entry(key.to_string())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("workflow entries are arrays")
                .push(serde_json::json!({
                    "name": check.name,
                    "bucket": check.bucket,
                    "state": check.state,
                    "started_at": check.started_at,
                    "completed_at": check.completed_at,
                    "link": check.link,
                    "description": check.description,
                    "event": check.event,
                }));
        }

        let matrix = serde_json::json!({
            "pr": number,
            "conclusion": counts.conclusion(),
            "counts": counts,
            "workflows": workflows,
        });

        let uri = format!("skipper://pr/{number}/checks");
        let json = serde_json::to_string_pretty(&matrix).unwrap_or_else(|_| "{}".to_string());

        Ok(vec![text_resource_with_mime(uri, json, "application/json")])
    }
}
