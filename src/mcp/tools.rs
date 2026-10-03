#[cfg(any(feature = "github", feature = "gitlab"))]
pub mod build_status;
#[cfg(any(feature = "github", feature = "gitlab"))]
pub mod build_watch;
#[cfg(feature = "github")]
pub mod gh_repo_list;
#[cfg(feature = "gitlab")]
pub mod glab_project_list;
#[cfg(feature = "github")]
pub mod pr_build_wait;
#[cfg(feature = "github")]
pub mod pr_watch;
#[cfg(feature = "tea")]
pub mod repo_search;

use mcp_host::prelude::*;
#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
use schemars::JsonSchema;
#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
use serde::Deserialize;
use std::sync::Arc;

#[cfg(any(feature = "github", feature = "gitlab"))]
use crate::provider::Provider;
#[cfg(any(feature = "github", feature = "gitlab"))]
use crate::provider::Registry;

pub struct SkipperServer {
    #[cfg(any(feature = "github", feature = "gitlab"))]
    pub(crate) registry: Arc<Registry>,
    #[cfg(feature = "tea")]
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) env: Arc<crate::environment::SkipperEnvironment>,
    #[cfg(feature = "github")]
    pub(crate) pr_watcher: Arc<crate::pr_watch::PrWatcher>,
}

#[cfg(any(feature = "github", feature = "gitlab"))]
pub(super) const CI_PROVIDERS: &[&str] = &["github", "gitlab"];

#[cfg(any(feature = "github", feature = "gitlab"))]
impl SkipperServer {
    pub(super) fn ci_provider(
        &self,
        explicit: Option<&str>,
    ) -> Result<Arc<dyn Provider>, ToolError> {
        use crate::environment::Environment as _;

        let forges = self.env.forges();
        let usable = |name: &str| self.registry.is_enabled(name) && forges.contains(name);

        match explicit {
            Some(name) => {
                if !CI_PROVIDERS.contains(&name) {
                    return Err(ToolError::InvalidArguments(format!(
                        "provider must be one of: {}",
                        CI_PROVIDERS.join(", ")
                    )));
                }
                if !self.registry.is_enabled(name) {
                    return Err(ToolError::Execution(format!(
                        "provider {name} is not available (CLI missing or unauthenticated)"
                    )));
                }
                if !forges.contains(name) {
                    return Err(ToolError::Execution(format!(
                        "no remote of this workspace points at {name}"
                    )));
                }
                self.registry
                    .get_arc(name)
                    .ok_or_else(|| ToolError::Internal(format!("provider {name} not registered")))
            }
            None => {
                let enabled: Vec<&str> =
                    CI_PROVIDERS.iter().copied().filter(|name| usable(name)).collect();
                match enabled[..] {
                    [name] => self.ci_provider(Some(name)),
                    [] => Err(ToolError::Execution(
                        "no CI-capable forge for this workspace (need a GitHub or GitLab \
                         remote with gh or glab authenticated)"
                            .to_string(),
                    )),
                    _ => Err(ToolError::InvalidArguments(format!(
                        "this workspace has remotes on several CI forges ({}), pass `provider`",
                        enabled.join(", ")
                    ))),
                }
            }
        }
    }
}

#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
pub(super) fn cli_error(e: crate::error::CliError) -> ToolError {
    ToolError::Execution(e.to_string())
}

#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
pub(super) fn json_output<T: serde::Serialize>(value: &T) -> ToolResult {
    let json = serde_json::to_string(value)
        .map_err(|e| ToolError::Internal(format!("serialization failed: {e}")))?;
    Ok(ToolOutput::text(json))
}

#[must_use]
pub fn router() -> McpRouter<SkipperServer> {
    #[allow(unused_mut)]
    let mut tools = McpToolRouter::new();

    #[cfg(feature = "github")]
    {
        tools = tools
            .with_tool(
                SkipperServer::gh_repo_list_tool_info(),
                SkipperServer::gh_repo_list_handler,
                Some(SkipperServer::gh_repo_list_visibility),
            )
            .with_tool(
                SkipperServer::pr_build_wait_tool_info(),
                SkipperServer::pr_build_wait_handler,
                Some(SkipperServer::pr_build_wait_visibility),
            )
            .with_tool(
                SkipperServer::pr_watch_tool_info(),
                SkipperServer::pr_watch_handler,
                Some(SkipperServer::pr_watch_visibility),
            );
    }

    #[cfg(feature = "gitlab")]
    {
        tools = tools.with_tool(
            SkipperServer::glab_project_list_tool_info(),
            SkipperServer::glab_project_list_handler,
            Some(SkipperServer::glab_project_list_visibility),
        );
    }

    #[cfg(feature = "tea")]
    {
        tools = tools.with_tool(
            SkipperServer::repo_search_tool_info(),
            SkipperServer::repo_search_handler,
            Some(SkipperServer::repo_search_visibility),
        );
    }

    #[cfg(any(feature = "github", feature = "gitlab"))]
    {
        tools = tools
            .with_tool(
                SkipperServer::build_status_tool_info(),
                SkipperServer::build_status_handler,
                Some(SkipperServer::build_status_visibility),
            )
            .with_tool(
                SkipperServer::build_watch_tool_info(),
                SkipperServer::build_watch_handler,
                Some(SkipperServer::build_watch_visibility),
            );
    }

    #[allow(unused_mut)]
    let mut resources = McpResourceRouter::new().with_resource(
        SkipperServer::workspace_resource_info(),
        SkipperServer::workspace_handler,
        None,
    );

    #[cfg(feature = "github")]
    {
        resources = resources.with_resource(
            SkipperServer::watch_resource_info(),
            SkipperServer::watch_handler,
            Some(SkipperServer::watch_visibility),
        );
    }

    #[cfg(feature = "tea")]
    {
        resources = resources.with_resource(
            SkipperServer::repo_resource_info(),
            SkipperServer::repo_handler,
            Some(SkipperServer::repo_visibility),
        );
    }

    #[allow(unused_mut)]
    let mut templates = McpResourceTemplateRouter::new();

    #[cfg(feature = "github")]
    {
        templates = templates
            .with_template(
                SkipperServer::pr_checks_template_info(),
                SkipperServer::pr_checks_handler,
                None,
            )
            .with_template(
                SkipperServer::pr_comments_template_info(),
                SkipperServer::pr_comments_handler,
                None,
            )
            .with_template(
                SkipperServer::pr_comments_kind_template_info(),
                SkipperServer::pr_comments_kind_handler,
                None,
            )
            .with_template(
                SkipperServer::watch_comments_template_info(),
                SkipperServer::watch_comments_handler,
                Some(SkipperServer::watch_comments_visibility),
            )
            .with_template(
                SkipperServer::pr_list_template_info(),
                SkipperServer::pr_list_handler,
                None,
            );
    }

    #[cfg(any(feature = "github", feature = "tea"))]
    {
        templates = templates
            .with_template(
                SkipperServer::issues_template_info(),
                SkipperServer::issues_handler,
                Some(SkipperServer::issues_visibility),
            )
            .with_template(
                SkipperServer::issue_template_info(),
                SkipperServer::issue_handler,
                Some(SkipperServer::issue_visibility),
            );
    }

    McpRouter::new(tools, McpPromptRouter::new(), resources, templates)
}

// Every assertion is per forge; a forge-less build has nothing to check.
#[cfg(all(test, any(feature = "github", feature = "gitlab", feature = "tea")))]
mod tests;
