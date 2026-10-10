use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, mcp_tool,
};
use crate::git::SyncOutcome;
use mcp_host::prelude::structured;
use serde::Serialize;

#[derive(Deserialize, JsonSchema)]
pub struct GitPushParams {
    /// Remotes to push to (default: the current remote); `["all"]` for every remote
    #[serde(default)]
    remotes: Vec<String>,
    /// Branches to push (default: the checked-out branch)
    #[serde(default)]
    refs: Vec<String>,
    /// Tags to push
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct GitPushResult {
    /// One entry per remote; a failed remote does not stop the others.
    pub pushed: Vec<SyncOutcome>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "git_push",
        description = "Push branches and tags to remotes. Never forces or deletes. Asks the user to approve unless confirm is off; check ok per remote",
        task_support = "optional",
        output = "GitPushResult",
        destructive = false,
        idempotent = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"writes\").is_some()).unwrap_or(false)"
    )]
    async fn git_push(&self, ctx: Ctx<'_>, params: Parameters<GitPushParams>) -> ToolResult {
        use crate::environment::Environment as _;

        let cwd = self.env.cwd();
        let plan = crate::git::plan_push(cwd, &params.0.remotes, &params.0.refs, &params.0.tags)
            .map_err(|e| ToolError::from(crate::git::tools::GitToolError::from(e)))?;
        self.confirm_write(&ctx, &plan.summary()).await?;
        let pushed = crate::git::push(cwd, &plan)
            .await
            .map_err(|e| ToolError::from(crate::git::tools::GitToolError::from(e)))?;
        structured(GitPushResult { pushed })
    }
}
