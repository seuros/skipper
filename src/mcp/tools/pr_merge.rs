use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, cli_error,
    mcp_tool,
};
use crate::provider::github::{GitHubProvider, PrOverview};
use mcp_host::prelude::structured;
use serde::Serialize;

#[derive(Deserialize, JsonSchema)]
pub struct PrMergeParams {
    /// PR number (default: the PR belonging to the current branch)
    pr: Option<u64>,
    /// Repo to read: a remote name or owner/name (default: the current remote)
    repo: Option<String>,
    /// merge | squash | rebase (default: the repo's default for this account)
    method: Option<String>,
    /// Refuse while checks fail or run (default: true)
    require_checks: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct PrMergeResult {
    pub pr: u64,
    pub method: String,
    /// The merge commit.
    pub sha: String,
}

impl SkipperServer {
    #[mcp_tool(
        name = "pr_merge",
        description = "Merge a GitHub PR at the head sha it was checked at. Refuses drafts, conflicts, and failing or running checks unless require_checks=false. Asks the user to approve unless confirm is off",
        output = "PrMergeResult",
        destructive = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:github\").is_some() && e.get_custom(\"writes\").is_some()).unwrap_or(false)"
    )]
    async fn pr_merge(&self, ctx: Ctx<'_>, params: Parameters<PrMergeParams>) -> ToolResult {
        let gh = GitHubProvider::new();
        let (repo, number) = gh
            .locate_pr(&self.env, params.0.repo.as_deref(), params.0.pr)
            .await
            .map_err(cli_error)?;
        let pr = gh.pr_overview(&repo, number).await.map_err(cli_error)?;
        let method = merge_method(&pr, params.0.method.as_deref())?;

        if params.0.require_checks.unwrap_or(true) {
            let counts = &pr.checks.counts;
            if counts.fail > 0 || counts.cancelled > 0 || counts.pending > 0 {
                return Err(ToolError::InvalidArguments(format!(
                    "#{number} checks are {}: {} failed, {} cancelled, {} pending; wait, fix, \
                     or pass require_checks=false",
                    pr.checks.conclusion, counts.fail, counts.cancelled, counts.pending
                )));
            }
        }

        self.confirm_write(
            &ctx,
            &format!(
                "{method} #{number} \"{}\" ({} → {}) in {}",
                pr.title,
                pr.head,
                pr.base,
                repo.full_name()
            ),
        )
        .await?;
        let sha = gh.merge_pr(&repo, number, &method, &pr.head_sha).await.map_err(cli_error)?;
        structured(PrMergeResult { pr: number, method, sha })
    }
}

/// The method to merge `pr` by, after refusing what GitHub would refuse anyway
/// with a less useful message.
fn merge_method(pr: &PrOverview, requested: Option<&str>) -> Result<String, ToolError> {
    let refuse = |why: String| Err(ToolError::InvalidArguments(format!("#{}: {why}", pr.pr)));
    if pr.state != "open" {
        return refuse(format!("is {}", pr.state));
    }
    if pr.draft {
        return refuse("is a draft".to_string());
    }
    if pr.mergeable == "conflicting" {
        return refuse(format!("conflicts with {}", pr.base));
    }
    let method = requested.map_or_else(|| pr.default_method.clone(), str::to_lowercase);
    if !pr.merge_methods.contains(&method) {
        return refuse(format!(
            "the repo does not allow {method}; allowed: {}",
            pr.merge_methods.join(", ")
        ));
    }
    Ok(method)
}

#[cfg(test)]
mod tests;
