use std::sync::LazyLock;

use mcp_host::prelude::*;

pub const BASE: &str = "Git and forge ops for this workspace: local git, repo metadata, CI runs, PR checks. \
     Issues, PRs (current remote): skipper://issues/{state}, skipper://issue/{number}, \
     skipper://pr/{number}, skipper://pr/{number}/comments. Prefer over git/gh/glab/tea CLIs.";

/// Instructions for the initializing client; `None` keeps [`BASE`].
pub fn for_client(ctx: &InstructionsContext<'_>) -> Option<String> {
    if model_sees_resources(ctx.client) {
        return None;
    }

    let catalog = ctx.resource_catalog()?;
    Some(format!("{BASE}\n\nResources (ReadMcpResourceTool; fill {{…}} in templates):\n{catalog}"))
}

// Client quirks, as mcp-host client specs: a `clientInfo.name`, optionally
// with version constraints (`claude-code < 2.93`) so a quirk ends at the
// release that fixes it.

/// Clients that never list MCP resources to their model: resources sit behind
/// ReadMcpResourceTool, so the instructions inline the catalog.
const HIDES_RESOURCES: &[&str] = &["claude-code"];

/// Clients that replace any failed resource read with their own text: the
/// error comes back as content so the model learns why.
pub const HIDES_RESOURCE_ERRORS: &[&str] = &["claude-code"];

fn model_sees_resources(client: &Implementation) -> bool {
    static HIDDEN: LazyLock<Vec<ClientMatcher>> = LazyLock::new(|| {
        HIDES_RESOURCES.iter().map(|spec| spec.parse().expect("valid client spec")).collect()
    });
    !HIDDEN.iter().any(|matcher| matcher.matches(client))
}
