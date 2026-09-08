use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolOutput, ToolResult,
    cli_error, mcp_tool,
};

#[derive(Deserialize, JsonSchema)]
pub struct GlabProjectListParams {
    /// Number of projects to list (default: 30)
    limit: Option<usize>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "glab_project_list",
        description = "List the account's GitLab projects. Account-wide, not this repo",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:gitlab\").is_some()).unwrap_or(false)"
    )]
    async fn glab_project_list(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<GlabProjectListParams>,
    ) -> ToolResult {
        let provider = self
            .registry
            .get("gitlab")
            .ok_or_else(|| ToolError::Execution("GitLab CLI not available".to_string()))?;

        let limit = params.0.limit.unwrap_or(30).to_string();
        let output = provider
            .execute(&["project", "list", "--output", "json", "--per-page", &limit])
            .await
            .map_err(cli_error)?;

        Ok(ToolOutput::text(output.stdout))
    }
}
