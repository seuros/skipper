use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolResult, cli_error, json_output,
    mcp_tool,
};

#[derive(Deserialize, JsonSchema)]
pub struct GhRepoListParams {
    /// Number of repos to list (default: 30)
    limit: Option<usize>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "gh_repo_list",
        description = "List the account's GitHub repos. Account-wide, not this repo",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:github\").is_some()).unwrap_or(false)"
    )]
    async fn gh_repo_list(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<GhRepoListParams>,
    ) -> ToolResult {
        self.require_github()?;
        let host = crate::workspace::forge_repo_on(&self.env, "github")
            .map_or_else(|_| "github.com".to_string(), |repo| repo.host);
        let repos = crate::provider::github::GitHubProvider::new()
            .owned_repos(&host, params.0.limit.unwrap_or(30))
            .await
            .map_err(cli_error)?;
        json_output(&repos)
    }
}
