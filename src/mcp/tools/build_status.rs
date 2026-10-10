use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolResult, cli_error, json_output,
    mcp_tool,
};

#[derive(Deserialize, JsonSchema)]
pub struct BuildStatusParams {
    /// Forge to query: "github" or "gitlab" (auto-detected when only one is enabled)
    provider: Option<String>,
    /// Specific run/pipeline id (default: recent runs for the current repository)
    run_id: Option<String>,
    /// Number of runs to list when no `run_id` is given (default: 10)
    limit: Option<usize>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "build_status",
        description = "Recent CI runs for this repo, or one by id. Non-blocking",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && (e.get_custom(\"forge:github\").is_some() || e.get_custom(\"forge:gitlab\").is_some())).unwrap_or(false)"
    )]
    async fn build_status(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<BuildStatusParams>,
    ) -> ToolResult {
        let provider = self.ci_provider(params.0.provider.as_deref())?;
        let target = provider.ci_target(&self.env).map_err(cli_error)?;

        if let Some(id) = params.0.run_id {
            let run = provider.ci_run(&self.env, &target, Some(&id)).await.map_err(cli_error)?;
            json_output(&run)
        } else {
            let runs =
                provider.ci_runs(&target, params.0.limit.unwrap_or(10)).await.map_err(cli_error)?;
            json_output(&runs)
        }
    }
}
