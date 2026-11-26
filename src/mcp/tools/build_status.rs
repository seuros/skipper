use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, cli_error, mcp_tool,
};
use crate::provider::BuildRun;
use serde::Serialize;

#[derive(Deserialize, JsonSchema)]
pub struct BuildStatusParams {
    /// Forge to query: "github" or "gitlab" (auto-detected when only one is enabled)
    provider: Option<String>,
    /// Specific run/pipeline id (default: recent runs for the current repository)
    run_id: Option<String>,
    /// Number of runs to list when no `run_id` is given (default: 10)
    limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("type" = "object"))]
pub enum BuildStatusResult {
    Run(BuildRun),
    Recent { runs: Vec<BuildRun> },
}

impl SkipperServer {
    #[mcp_tool(
        name = "build_status",
        description = "Recent CI runs for this repo, or one by id. Non-blocking",
        output = "BuildStatusResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && (e.has_custom(\"forge:github\") || e.has_custom(\"forge:gitlab\"))).unwrap_or(false)"
    )]
    async fn build_status(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<BuildStatusParams>,
    ) -> Result<BuildStatusResult, ToolError> {
        let provider = self.ci_provider(params.0.provider.as_deref())?;
        let target = provider.ci_target(&self.env).map_err(cli_error)?;

        if let Some(id) = params.0.run_id {
            let run = provider.ci_run(&self.env, &target, Some(&id)).await.map_err(cli_error)?;
            Ok(BuildStatusResult::Run(run))
        } else {
            let runs =
                provider.ci_runs(&target, params.0.limit.unwrap_or(10)).await.map_err(cli_error)?;
            Ok(BuildStatusResult::Recent { runs })
        }
    }
}
