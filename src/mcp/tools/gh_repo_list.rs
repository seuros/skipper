use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolOutput, ToolResult,
    cli_error, mcp_tool,
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
        let provider = self
            .registry
            .get("github")
            .ok_or_else(|| ToolError::Execution("GitHub CLI not available".to_string()))?;

        let limit = params.0.limit.unwrap_or(30).to_string();
        let output = provider
            .execute(&["repo", "list", "--json", "name,owner,description", "--limit", &limit])
            .await
            .map_err(cli_error)?;

        Ok(ToolOutput::text(output.stdout))
    }
}
