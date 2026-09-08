use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, ToolResult, cli_error,
    json_output, mcp_tool,
};
use crate::provider::forgejo::{ForgejoClient, any_credentials};
use serde::Serialize;

#[derive(Deserialize, JsonSchema)]
pub struct RepoSearchParams {
    /// Substring to match against repository names (omit to list all)
    query: Option<String>,
    /// Results per page (default: 20, max 50)
    limit: Option<u32>,
    /// 1-based page number (default: 1)
    page: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
pub struct RepoSearchResult {
    pub page: u32,
    pub limit: u32,
    pub count: usize,
    /// Present only when this page was full, so more may follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<u32>,
    pub repos: Vec<crate::provider::forgejo::Repository>,
}

impl SkipperServer {
    #[mcp_tool(
        name = "repo_search",
        description = "Search Gitea/Forgejo repos by name, paginated. For this repo use skipper://repo",
        output = "RepoSearchResult",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.get_custom(\"forge:tea\").is_some()).unwrap_or(false)"
    )]
    async fn repo_search(&self, _ctx: Ctx<'_>, params: Parameters<RepoSearchParams>) -> ToolResult {
        let creds = any_credentials().ok_or_else(|| {
            ToolError::Execution(
                "no Gitea/Forgejo login found; run `tea login add` to store one".to_string(),
            )
        })?;

        let limit = params.0.limit.unwrap_or(20).clamp(1, 50);
        let page = params.0.page.unwrap_or(1).max(1);

        let results = ForgejoClient::new(creds)
            .repo_search(params.0.query.as_deref(), limit, page)
            .await
            .map_err(cli_error)?;

        let repos = results.data;
        let next_page = (repos.len() as u32 == limit).then_some(page + 1);

        json_output(&RepoSearchResult { page, limit, count: repos.len(), next_page, repos })
    }
}
