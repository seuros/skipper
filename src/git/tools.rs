use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mcp_host::prelude::*;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::git::GitCtx;
use crate::git::GitServer;

const GIT_TOOL_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a git tool call failed. Caller errors (a bad ref, nothing staged) are
/// the request's fault: they answer `InvalidArguments`, which does not trip
/// the tool's circuit breaker.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitToolError {
    #[error("{0}")]
    Caller(String),
    #[error("{0}")]
    Failed(String),
}

impl From<crate::git::GitError> for GitToolError {
    fn from(e: crate::git::GitError) -> Self {
        if e.is_caller_error() { Self::Caller(e.to_string()) } else { Self::Failed(e.to_string()) }
    }
}

impl From<GitToolError> for ToolError {
    fn from(e: GitToolError) -> Self {
        match e {
            GitToolError::Caller(message) => Self::InvalidArguments(message),
            GitToolError::Failed(message) => Self::Execution(message),
        }
    }
}

pub(crate) async fn execute_blocking<P, F, R>(
    cwd: PathBuf,
    params: P,
    f: F,
) -> Result<R, GitToolError>
where
    P: Send + 'static,
    F: FnOnce(&Path, P) -> Result<R, GitToolError> + Send + 'static,
    R: Send + 'static,
{
    let task = tokio::task::spawn_blocking(move || f(&cwd, params));
    tokio::time::timeout(GIT_TOOL_TIMEOUT, task)
        .await
        .map_err(|_| GitToolError::Failed("git tool timed out".to_string()))?
        .map_err(|e| GitToolError::Failed(format!("git tool task failed: {e}")))?
}

async fn execute_cancellable_blocking<P, F, R>(
    cwd: PathBuf,
    params: P,
    timeout: Duration,
    operation: &'static str,
    f: F,
) -> Result<R, GitToolError>
where
    P: Send + 'static,
    F: FnOnce(&Path, P, &Arc<AtomicBool>) -> Result<R, GitToolError> + Send + 'static,
    R: Send + 'static,
{
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let mut task = tokio::task::spawn_blocking(move || f(&cwd, params, &worker_cancel));
    tokio::select! {
        biased;
        result = &mut task => {
            result.map_err(|e| GitToolError::Failed(format!("{operation} task failed: {e}")))?
        }
        () = tokio::time::sleep(timeout) => {
            cancel.store(true, Ordering::Release);
            let _worker_result = task.await.map_err(|e| {
                GitToolError::Failed(format!(
                    "{operation} timed out after {timeout:?}; worker cleanup failed: {e}"
                ))
            })?;
            Err(GitToolError::Failed(format!("{operation} timed out after {timeout:?}")))
        }
    }
}

pub(crate) async fn execute_git_diff_blocking(
    cwd: PathBuf,
    params: GitDiffParams,
) -> Result<serde_json::Value, GitToolError> {
    execute_cancellable_blocking(
        cwd,
        params,
        GIT_TOOL_TIMEOUT,
        "git diff",
        execute_git_diff_structured_with_cancel,
    )
    .await
}

/// The tool's object as structured output, as is: `ToolOutput::structured`
/// would serialize the finished `Value` into a second copy.
fn output_from_json_result(result: Result<Value, GitToolError>) -> ToolResult {
    match result? {
        Value::Object(map) => Ok(ToolOutput::json(map)),
        _ => Err(ToolError::Execution("non-object tool output".to_string())),
    }
}

/// A JSON object of owned values, moved in. `json!` serializes every value
/// through a reference, copying strings and whole subtrees.
fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
}

fn line_range(
    start: Option<usize>,
    end: Option<usize>,
) -> Result<Option<(usize, usize)>, GitToolError> {
    match (start, end) {
        (Some(start), Some(end)) => Ok(Some((start, end))),
        (None, None) => Ok(None),
        _ => Err(GitToolError::Caller(
            "start_line and end_line must either both be provided or both be omitted".to_string(),
        )),
    }
}

fn to_json_value<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value, GitToolError> {
    serde_json::to_value(value).map_err(|error| GitToolError::Failed(error.to_string()))
}

const fn default_log_limit() -> usize {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitDiffParams {
    /// Comparison scope: worktree is index-to-filesystem, staged is base-to-index,
    /// and all is base-to-filesystem. Untracked files are excluded.
    scope: crate::git::DiffScope,
    /// Output representation.
    format: crate::git::DiffFormat,
    /// Check newly added lines for whitespace errors and conflict markers.
    check: bool,
    /// Optional base ref for staged or all scope (default: HEAD). Invalid with worktree.
    #[serde(default)]
    base: Option<String>,
    /// Optional exact file or directory-prefix filters relative to repo root.
    #[serde(default)]
    paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitLogParams {
    /// Maximum number of entries to return.
    #[serde(default = "default_log_limit")]
    limit: usize,
    /// Optional ref to walk from (default: HEAD).
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitShowParams {
    /// Revision to show (default: HEAD).
    #[serde(default)]
    rev: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitShowFileParams {
    /// File path relative to repo root.
    file_path: String,
    /// Revision to read (default: HEAD).
    #[serde(default)]
    rev: Option<String>,
    /// Optional 1-indexed start line, inclusive.
    #[serde(default)]
    start_line: Option<usize>,
    /// Optional 1-indexed end line, inclusive.
    #[serde(default)]
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitBlameParams {
    /// File path relative to repo root.
    file_path: String,
    /// Optional 1-indexed start line, inclusive.
    #[serde(default)]
    start_line: Option<usize>,
    /// Optional 1-indexed end line, inclusive.
    #[serde(default)]
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitRepoParams {}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitStatusParams {}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitRemotesParams {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitAddParams {
    /// Explicit repository-relative file paths to stage.
    paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitCommitParams {
    /// Commit message. The first line becomes the commit subject.
    message: String,
    /// Structured trailers appended to the stored commit message in order.
    #[serde(default)]
    trailers: Vec<crate::git::CommitTrailer>,
    /// Replace HEAD instead of creating a child commit. Staged changes, if any,
    /// are included; with no staged changes, only the commit message changes.
    #[serde(default)]
    amend: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GitBranchOperation {
    Create,
    Delete,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GitBranchParams {
    /// Branch operation to perform.
    operation: GitBranchOperation,
    /// Short local branch name, without the refs/heads/ prefix.
    name: String,
    /// Revision used as the new branch tip. Create only; defaults to HEAD.
    #[serde(default)]
    start_point: Option<String>,
    /// Allow deletion when the branch is not merged into HEAD. Delete only.
    #[serde(default)]
    force: bool,
}

impl GitServer {
    #[mcp_tool(
        name = "git_diff",
        description = "Inspect tracked changes by scope, returning structured patches, statistics, or changed paths with optional whitespace checks.",
        read_only = true,
        open_world = false
    )]
    async fn git_diff(&self, _ctx: GitCtx<'_>, params: Parameters<GitDiffParams>) -> ToolResult {
        output_from_json_result(execute_git_diff_blocking(PathBuf::from("."), params.0).await)
    }

    #[mcp_tool(
        name = "git_log",
        description = "List recent commits with sha, author, date, and subject.",
        read_only = true,
        open_world = false
    )]
    async fn git_log(&self, _ctx: GitCtx<'_>, params: Parameters<GitLogParams>) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_log_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_show",
        description = "Show full commit details including subject, body, author, and trailers.",
        read_only = true,
        open_world = false
    )]
    async fn git_show(&self, _ctx: GitCtx<'_>, params: Parameters<GitShowParams>) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_show_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_show_file",
        description = "Show a tracked file at a revision, with optional line range. Untracked worktree edits are ignored.",
        read_only = true,
        open_world = false
    )]
    async fn git_show_file(
        &self,
        _ctx: GitCtx<'_>,
        params: Parameters<GitShowFileParams>,
    ) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_show_file_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_blame",
        description = "Show per-line author attribution for a file, with optional line range.",
        read_only = true,
        open_world = false
    )]
    async fn git_blame(&self, _ctx: GitCtx<'_>, params: Parameters<GitBlameParams>) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_blame_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_repo",
        description = "Show repository identity: root path, HEAD sha, current branch, remotes, and dirty state.",
        read_only = true,
        open_world = false
    )]
    async fn git_repo(&self, _ctx: GitCtx<'_>, params: Parameters<GitRepoParams>) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_repo_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_status",
        description = "List staged, unstaged, and untracked files in the worktree.",
        read_only = true,
        open_world = false
    )]
    async fn git_status(
        &self,
        _ctx: GitCtx<'_>,
        params: Parameters<GitStatusParams>,
    ) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_status_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_remotes",
        description = "List configured remotes with their fetch and push URLs.",
        read_only = true,
        open_world = false
    )]
    async fn git_remotes(
        &self,
        _ctx: GitCtx<'_>,
        params: Parameters<GitRemotesParams>,
    ) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_remotes_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_add",
        description = "Stage explicit repository-relative files or deletions. Directories, ignored new files, conflicts, and submodules are rejected.",
        read_only = false,
        destructive = false,
        open_world = false
    )]
    async fn git_add(&self, _ctx: GitCtx<'_>, params: Parameters<GitAddParams>) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_add_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_commit",
        description = "Create or amend an unsigned commit, with optional structured trailers. Git hooks are not run.",
        read_only = false,
        destructive = true,
        open_world = false
    )]
    async fn git_commit(
        &self,
        _ctx: GitCtx<'_>,
        params: Parameters<GitCommitParams>,
    ) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_commit_structured).await,
        )
    }

    #[mcp_tool(
        name = "git_branch",
        description = "Create or delete a local branch. Creation does not check out the branch; deletion rejects checked-out or unmerged branches unless force is set.",
        read_only = false,
        destructive = true,
        open_world = false
    )]
    async fn git_branch(
        &self,
        _ctx: GitCtx<'_>,
        params: Parameters<GitBranchParams>,
    ) -> ToolResult {
        output_from_json_result(
            execute_blocking(PathBuf::from("."), params.0, execute_git_branch_structured).await,
        )
    }
}

pub fn tool_infos() -> Vec<ToolInfo> {
    vec![
        GitServer::git_diff_tool_info(),
        GitServer::git_log_tool_info(),
        GitServer::git_show_tool_info(),
        GitServer::git_show_file_tool_info(),
        GitServer::git_blame_tool_info(),
        GitServer::git_repo_tool_info(),
        GitServer::git_status_tool_info(),
        GitServer::git_remotes_tool_info(),
        GitServer::git_add_tool_info(),
        GitServer::git_commit_tool_info(),
        GitServer::git_branch_tool_info(),
    ]
}

fn execute_git_diff_structured_with_cancel(
    cwd: &Path,
    GitDiffParams { scope, format, check, base, paths }: GitDiffParams,
    cancel: &Arc<AtomicBool>,
) -> Result<serde_json::Value, GitToolError> {
    let report_scope = scope;
    let path_refs =
        paths.as_ref().map(|items| items.iter().map(String::as_str).collect::<Vec<_>>());
    let report = crate::git::diff::diff_report_with_cancel(
        cwd,
        report_scope,
        format,
        base.as_deref(),
        path_refs.as_deref(),
        check,
        cancel,
    )?;
    // Built by move: a patch is up to 8 MiB, and `json!` would copy it once
    // per level of nesting.
    let result = match format {
        crate::git::DiffFormat::Patch => object([
            ("format", json!(format)),
            (
                "files",
                report
                    .files
                    .into_iter()
                    .map(|file| {
                        object([
                            ("path", file.path.into()),
                            ("status", json!(file.status)),
                            ("binary", file.binary.into()),
                            ("patch", file.patch.into()),
                        ])
                    })
                    .collect(),
            ),
        ]),
        crate::git::DiffFormat::Stat => object([
            ("format", json!(format)),
            (
                "files",
                report
                    .files
                    .into_iter()
                    .map(|file| {
                        object([
                            ("path", file.path.into()),
                            ("status", json!(file.status)),
                            ("binary", file.binary.into()),
                            ("additions", file.additions.into()),
                            ("deletions", file.deletions.into()),
                        ])
                    })
                    .collect(),
            ),
        ]),
        crate::git::DiffFormat::NameOnly => {
            object([("format", json!(format)), ("paths", report.paths.into())])
        }
    };
    let whitespace_check = if check {
        object([
            ("checked", true.into()),
            ("passed", report.whitespace_errors.is_empty().into()),
            ("errors", to_json_value(&report.whitespace_errors)?),
        ])
    } else {
        object([
            ("checked", false.into()),
            ("passed", Value::Null),
            ("errors", Value::Array(Vec::new())),
        ])
    };
    let resolved_base = match report_scope {
        crate::git::DiffScope::Worktree => None,
        crate::git::DiffScope::Staged | crate::git::DiffScope::All => {
            Some(base.as_deref().unwrap_or("HEAD"))
        }
    };
    Ok(object([
        ("scope", json!(report_scope)),
        ("base", resolved_base.into()),
        ("path_filters", paths.unwrap_or_default().into()),
        ("summary", to_json_value(&report.summary)?),
        ("result", result),
        ("whitespace_check", whitespace_check),
    ]))
}

pub fn execute_git_log_structured(
    cwd: &Path,
    GitLogParams { limit, branch }: GitLogParams,
) -> Result<serde_json::Value, GitToolError> {
    let entries = crate::git::log(cwd, Some(limit), branch.as_deref())?;
    Ok(object([("commits", to_json_value(&entries)?)]))
}

pub fn execute_git_show_structured(
    cwd: &Path,
    GitShowParams { rev }: GitShowParams,
) -> Result<serde_json::Value, GitToolError> {
    let entry = crate::git::show(cwd, rev.as_deref())?;
    to_json_value(&entry)
}

pub fn execute_git_show_file_structured(
    cwd: &Path,
    GitShowFileParams { file_path, rev, start_line, end_line }: GitShowFileParams,
) -> Result<serde_json::Value, GitToolError> {
    let lines = line_range(start_line, end_line)?;
    let mut file = crate::git::show_file(cwd, &file_path, rev.as_deref(), lines)?;
    // The content (up to a whole blob) moves in rather than being serialized.
    let content = std::mem::take(&mut file.content);
    let mut value = to_json_value(&file)?;
    if let Value::Object(map) = &mut value {
        map.insert("content".to_owned(), Value::String(content));
    }
    Ok(value)
}

pub fn execute_git_blame_structured(
    cwd: &Path,
    GitBlameParams { file_path, start_line, end_line }: GitBlameParams,
) -> Result<serde_json::Value, GitToolError> {
    let lines = line_range(start_line, end_line)?;
    let blamed = crate::git::blame(cwd, &file_path, lines)?;
    Ok(object([("lines", to_json_value(&blamed)?)]))
}

pub fn execute_git_repo_structured(
    cwd: &Path,
    GitRepoParams {}: GitRepoParams,
) -> Result<serde_json::Value, GitToolError> {
    let info = crate::git::repo_info(cwd)?;
    to_json_value(&info)
}

pub fn execute_git_status_structured(
    cwd: &Path,
    GitStatusParams {}: GitStatusParams,
) -> Result<serde_json::Value, GitToolError> {
    let info = crate::git::status(cwd)?;
    to_json_value(&info)
}

pub fn execute_git_remotes_structured(
    cwd: &Path,
    GitRemotesParams {}: GitRemotesParams,
) -> Result<serde_json::Value, GitToolError> {
    let info = crate::git::remotes(cwd)?;
    to_json_value(&info)
}

pub fn execute_git_add_structured(
    cwd: &Path,
    GitAddParams { paths }: GitAddParams,
) -> Result<serde_json::Value, GitToolError> {
    let result = crate::git::add(cwd, &paths)?;
    to_json_value(&result)
}

pub fn execute_git_commit_structured(
    cwd: &Path,
    GitCommitParams { message, trailers, amend }: GitCommitParams,
) -> Result<serde_json::Value, GitToolError> {
    let result = if amend {
        crate::git::amend_with_trailers(cwd, &message, &trailers)
    } else {
        crate::git::commit_with_trailers(cwd, &message, &trailers)
    }?;
    to_json_value(&result)
}

pub fn execute_git_branch_structured(
    cwd: &Path,
    GitBranchParams { operation, name, start_point, force }: GitBranchParams,
) -> Result<serde_json::Value, GitToolError> {
    let result = match operation {
        GitBranchOperation::Create => {
            if force {
                return Err(GitToolError::Caller(
                    "force is only valid for branch deletion".to_string(),
                ));
            }
            crate::git::create_branch(cwd, &name, start_point.as_deref())
        }
        GitBranchOperation::Delete => {
            if start_point.is_some() {
                return Err(GitToolError::Caller(
                    "start_point is only valid for branch creation".to_string(),
                ));
            }
            crate::git::delete_branch(cwd, &name, force)
        }
    }?;
    to_json_value(&result)
}

#[cfg(test)]
mod tests;
