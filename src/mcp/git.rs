use crate::git::GitServer;
use mcp_host::prelude::*;

fn in_repo(ctx: &VisibilityContext) -> bool {
    ctx.environment.map(|e| e.has_git_repo()).unwrap_or(false)
}

fn dirty_tree(ctx: &VisibilityContext) -> bool {
    ctx.environment.map(|e| e.has_git_repo() && !e.git_is_clean()).unwrap_or(false)
}

fn has_staged(ctx: &VisibilityContext) -> bool {
    ctx.environment.map(|e| e.has_git_repo() && e.git_has_staged()).unwrap_or(false)
}

#[must_use]
pub fn router() -> McpRouter<GitServer> {
    let tools = McpToolRouter::new()
        .with_tool(GitServer::git_status_tool_info(), GitServer::git_status_handler, Some(in_repo))
        .with_tool(GitServer::git_log_tool_info(), GitServer::git_log_handler, Some(in_repo))
        .with_tool(GitServer::git_diff_tool_info(), GitServer::git_diff_handler, Some(in_repo))
        .with_tool(GitServer::git_show_tool_info(), GitServer::git_show_handler, Some(in_repo))
        .with_tool(
            GitServer::git_show_file_tool_info(),
            GitServer::git_show_file_handler,
            Some(in_repo),
        )
        .with_tool(GitServer::git_blame_tool_info(), GitServer::git_blame_handler, Some(in_repo))
        .with_tool(GitServer::git_branch_tool_info(), GitServer::git_branch_handler, Some(in_repo))
        .with_tool(GitServer::git_add_tool_info(), GitServer::git_add_handler, Some(dirty_tree))
        .with_tool(
            GitServer::git_commit_tool_info(),
            GitServer::git_commit_handler,
            Some(has_staged),
        );

    McpRouter::new(
        tools,
        McpPromptRouter::new(),
        McpResourceRouter::new(),
        McpResourceTemplateRouter::new(),
    )
}

#[cfg(test)]
mod tests;
