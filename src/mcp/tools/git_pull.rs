use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, mcp_tool,
};
use crate::git::SyncOutcome;
use mcp_host::prelude::structured;

#[derive(Deserialize, JsonSchema)]
pub struct GitPullParams {}

impl SkipperServer {
    #[mcp_tool(
        name = "git_pull",
        description = "Fast-forward the checked-out branch to its upstream. Never merges or rebases; a diverged branch fails",
        task_support = "optional",
        output = "SyncOutcome",
        destructive = false,
        idempotent = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"writes\").is_some()).unwrap_or(false)"
    )]
    async fn git_pull(&self, _ctx: Ctx<'_>, _params: Parameters<GitPullParams>) -> ToolResult {
        use crate::environment::Environment as _;

        let pulled: SyncOutcome = crate::git::pull(self.env.cwd())
            .await
            .map_err(|e| ToolError::from(crate::git::tools::GitToolError::from(e)))?;
        structured(pulled)
    }
}
