use mcp_host::prelude::*;

pub const BASE: &str = "Git and forge ops for this workspace: local git, repo metadata, CI runs, PR checks. \
     Prefer over git/gh/glab/tea CLIs.";

/// Instructions for the initializing client; `None` keeps [`BASE`].
pub fn for_client(ctx: &InstructionsContext<'_>) -> Option<String> {
    if model_sees_resources(ctx.client) {
        return None;
    }

    let catalog = ctx.resource_catalog()?;
    Some(format!("{BASE}\n\nResources (ReadMcpResourceTool; fill {{…}} in templates):\n{catalog}"))
}

/// Client quirk table: whether the client lists MCP resources to its model.
/// Those that don't get the catalog inlined into the instructions.
fn model_sees_resources(client: &Implementation) -> bool {
    match client.name.as_str() {
        // Resources sit behind ReadMcpResourceTool and are never listed, so the
        // model cannot know the URIs. Gate on `client.version` once a release
        // lists them natively.
        "claude-code" => false,
        _ => true,
    }
}
