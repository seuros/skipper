use super::{
    Ctx, Deserialize, JsonSchema, Parameters, SkipperServer, ToolError, cli_error, mcp_tool,
};
use crate::error::Result;
use crate::provider::github::{DatedRun, GitHubProvider, OpenPr, PrState};
use crate::watch_until::{Forge, Outcome, Step};
use crate::workspace::ForgeRepo;
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
pub struct WatchUntilParams {
    /// Conditions to wait for, in order (1 to 6)
    steps: Vec<Step>,
    /// Repo to read: a remote name or owner/name (default: the current remote)
    repo: Option<String>,
    /// Max seconds for the whole chain (default: 3600, clamped to 30..=14400)
    wait_secs: Option<u64>,
}

struct GithubForge {
    gh: GitHubProvider,
    repo: ForgeRepo,
}

impl Forge for GithubForge {
    async fn open_prs(&self) -> Result<Vec<OpenPr>> {
        self.gh.open_prs(&self.repo).await
    }

    async fn pr_state(&self, number: u64) -> Result<PrState> {
        self.gh.pr_state(&self.repo, number).await
    }

    async fn runs(&self, branch: Option<&str>) -> Result<Vec<DatedRun>> {
        self.gh.dated_runs(&self.repo, branch).await
    }
}

impl SkipperServer {
    #[mcp_tool(
        name = "watch_until",
        description = "Wait for a chain of GitHub conditions in order: a PR opening (it need not exist yet), a PR merging, a workflow run finishing. One call covers push -> release PR -> merge -> build. Returns each step's outcome; expired names the step still waiting, failed the one that broke the chain. Long-running; prefer task execution",
        task_support = "optional",
        output = "Outcome",
        read_only = true,
        open_world = true,
        visible = "ctx.environment.map(|e| e.has_git_repo() && e.has_custom(\"forge:github\")).unwrap_or(false)"
    )]
    async fn watch_until(
        &self,
        _ctx: Ctx<'_>,
        params: Parameters<WatchUntilParams>,
    ) -> std::result::Result<Outcome, ToolError> {
        self.require_github()?;
        let params = params.0;
        crate::watch_until::validate(&params.steps).map_err(ToolError::InvalidArguments)?;

        let repo = crate::workspace::select_on(&self.env, "github", params.repo.as_deref())
            .map_err(|e| cli_error(e.into()))?;
        let wait = Duration::from_secs(params.wait_secs.unwrap_or(3600).clamp(30, 14_400));
        let forge = GithubForge { gh: GitHubProvider::new(), repo };

        Ok(crate::watch_until::run(&forge, &params.steps, wait, jiff::Timestamp::now()).await)
    }
}
