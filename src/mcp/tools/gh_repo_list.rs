use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, cli_error, mcp_tool,
};
use crate::provider::github::OwnedRepo;
use serde::Serialize;

#[derive(Deserialize, JsonSchema)]
pub struct GhRepoListParams {
    /// Number of repos to list (default: 30)
    limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct GhRepoListResult {
    pub repos: Vec<OwnedRepo>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "gh_repo_list",
        description = "List the account's GitHub repos. Account-wide, not this repo",
        output = "GhRepoListResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.has_custom(\"forge:github\")).unwrap_or(false)"
    )]
    async fn gh_repo_list(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<GhRepoListParams>,
    ) -> Result<GhRepoListResult, ToolError> {
        self.require_github()?;
        let host = crate::workspace::forge_repo_on(&self.env, "github")
            .map_or_else(|_| "github.com".to_string(), |repo| repo.host);
        let repos = crate::provider::github::GitHubProvider::new()
            .owned_repos(&host, params.0.limit.unwrap_or(30))
            .await
            .map_err(cli_error)?;
        Ok(GhRepoListResult { repos })
    }
}
