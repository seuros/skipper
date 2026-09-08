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
    /// Number of runs to list when no run_id is given (default: 10)
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

        match params.0.run_id {
            Some(id) => {
                let run = provider.ci_run(Some(&id)).await.map_err(cli_error)?;
                json_output(&run)
            }
            None => {
                let runs =
                    provider.ci_runs(params.0.limit.unwrap_or(10)).await.map_err(cli_error)?;
                json_output(&runs)
            }
        }
    }
}
