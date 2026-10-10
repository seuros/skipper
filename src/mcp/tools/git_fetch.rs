use super::{Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolResult, mcp_tool};
use crate::git::SyncOutcome;
use mcp_host::prelude::structured;
use serde::Serialize;

#[derive(Deserialize, JsonSchema)]
pub struct GitFetchParams {
    /// Remote to fetch (default: the current remote); "all" for every remote
    remote: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct GitFetchResult {
    pub fetched: Vec<SyncOutcome>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "git_fetch",
        description = "Fetch a remote's branches and tags, pruning deleted branches. Updates remote-tracking refs only",
        task_support = "optional",
        output = "GitFetchResult",
        destructive = false,
        idempotent = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"writes\").is_some()).unwrap_or(false)"
    )]
    async fn git_fetch(&self, _ctx: Ctx<'_>, params: Parameters<GitFetchParams>) -> ToolResult {
        use crate::environment::Environment as _;

        let fetched = crate::git::fetch(self.env.cwd(), params.0.remote.as_deref()).await?;
        structured(GitFetchResult { fetched })
    }
}
