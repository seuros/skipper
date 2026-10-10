#[cfg(any(feature = "github", feature = "gitlab"))]
pub mod build_status;
#[cfg(any(feature = "github", feature = "gitlab"))]
pub mod build_watch;
#[cfg(feature = "github")]
pub mod gh_repo_list;
pub mod git_fetch;
pub mod git_pull;
pub mod git_push;
#[cfg(feature = "gitlab")]
pub mod glab_project_list;
#[cfg(feature = "github")]
pub mod pr_build_wait;
#[cfg(feature = "github")]
pub mod pr_merge;
#[cfg(feature = "github")]
pub mod pr_watch;
#[cfg(feature = "tea")]
pub mod repo_search;

use mcp_host::prelude::*;
use schemars::JsonSchema;
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
    pub(crate) writes: crate::config::WritesConfig,
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
        let usable = |name: &&str| self.registry.is_enabled(name) && self.env.has_forge(name);

        if let Some(name) = explicit {
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
            if !self.env.has_forge(name) {
                return Err(ToolError::Execution(format!(
                    "no remote of this workspace points at {name}"
                )));
            }
            self.registry
                .get_arc(name)
                .ok_or_else(|| ToolError::Internal(format!("provider {name} not registered")))
        } else {
            let mut enabled = CI_PROVIDERS.iter().copied().filter(usable);
            match (enabled.next(), enabled.next()) {
                (Some(name), None) => self.ci_provider(Some(name)),
                (None, _) => Err(ToolError::Execution(
                    "no CI-capable forge for this workspace (need a GitHub or GitLab \
                     remote with gh or glab authenticated)"
                        .to_string(),
                )),
                (Some(_), Some(_)) => Err(ToolError::InvalidArguments(format!(
                    "this workspace has remotes on several CI forges ({}), pass `provider`",
                    CI_PROVIDERS.iter().copied().filter(usable).collect::<Vec<_>>().join(", ")
                ))),
            }
        }
    }
}

impl SkipperServer {
    /// Ask the user to approve the write `summary` describes, unless the
    /// global config sets `[writes] confirm = false`. A declined write answers
    /// `InvalidArguments`: the user's call, not a failure to trip a breaker.
    pub(super) async fn confirm_write(
        &self,
        ctx: &Ctx<'_>,
        summary: &str,
    ) -> Result<(), ToolError> {
        use mcp_host::prelude::MultiplexerError;
        use mcp_host::protocol::types::ElicitationAction;
        use std::time::Duration;

        if !self.writes.confirm {
            return Ok(());
        }
        let unsupported = || {
            ToolError::Execution(
                "this client cannot ask for confirmation (no MCP elicitation); set `[writes] \
                 confirm = false` in ~/.config/skipper/config.toml to allow unattended writes"
                    .to_string(),
            )
        };
        let requester = ctx.client_requester().ok_or_else(unsupported)?;
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "approve": { "type": "boolean", "title": "Approve", "description": summary }
            },
            "required": ["approve"]
        });
        let answer = requester
            .request_elicitation(summary.to_string(), schema, Some(Duration::from_secs(300)))
            .await
            .map_err(|e| match e {
                MultiplexerError::UnsupportedCapability(_) => unsupported(),
                e => ToolError::Execution(format!("confirmation failed: {e}")),
            })?;
        let approved = answer.action == ElicitationAction::Accept
            && answer.content.as_ref().and_then(|c| c["approve"].as_bool()) == Some(true);
        if approved {
            Ok(())
        } else {
            Err(ToolError::InvalidArguments(format!("declined by the user: {summary}")))
        }
    }
}

#[cfg(any(feature = "github", feature = "gitlab", feature = "tea"))]
/// A forge failure as a tool error. A request skipper cannot serve (no PR for
/// the branch, no matching remote) is the caller's to fix: `InvalidArguments`,
/// which does not trip the tool's circuit breaker.
pub(super) fn cli_error(e: crate::error::CliError) -> ToolError {
    match e {
        crate::error::CliError::NoTarget(message) => ToolError::InvalidArguments(message),
        e => ToolError::Execution(e.to_string()),
    }
}

#[cfg(any(feature = "github", feature = "gitlab"))]
pub(super) fn json_output<T: serde::Serialize>(value: &T) -> ToolResult {
    let json = serde_json::to_string(value)
        .map_err(|e| ToolError::Internal(format!("serialization failed: {e}")))?;
    Ok(ToolOutput::text(json))
}

#[must_use]
pub fn router() -> McpRouter<SkipperServer> {
    McpRouter::new(tool_router(), McpPromptRouter::new(), resource_router(), template_router())
}

fn tool_router() -> McpToolRouter<SkipperServer> {
    #[allow(unused_mut)]
    let mut tools = McpToolRouter::new()
        .with_tool(
            SkipperServer::git_fetch_tool_info(),
            SkipperServer::git_fetch_handler,
            Some(SkipperServer::git_fetch_visibility),
        )
        .with_tool(
            SkipperServer::git_pull_tool_info(),
            SkipperServer::git_pull_handler,
            Some(SkipperServer::git_pull_visibility),
        )
        .with_tool(
            SkipperServer::git_push_tool_info(),
            SkipperServer::git_push_handler,
            Some(SkipperServer::git_push_visibility),
        );

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
            )
            .with_tool(
                SkipperServer::pr_merge_tool_info(),
                SkipperServer::pr_merge_handler,
                Some(SkipperServer::pr_merge_visibility),
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

    tools
}

fn resource_router() -> McpResourceRouter<SkipperServer> {
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

    resources
}

fn template_router() -> McpResourceTemplateRouter<SkipperServer> {
    #[allow(unused_mut)]
    let mut templates = McpResourceTemplateRouter::new();

    #[cfg(feature = "github")]
    {
        templates = templates
            .with_template(SkipperServer::pr_template_info(), SkipperServer::pr_handler, None)
            .with_template(
                SkipperServer::pr_files_template_info(),
                SkipperServer::pr_files_handler,
                None,
            )
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

    templates
}

#[cfg(all(test, feature = "github"))]
mod tests;
