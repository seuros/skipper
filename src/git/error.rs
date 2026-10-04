#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("not a git repository: {0}")]
    NotARepo(String),

    #[error("git operation failed: {0}")]
    Operation(String),

    #[error("reference not found: {0}")]
    RefNotFound(String),

    #[error("path not found: {0}")]
    PathNotFound(String),

    #[error("invalid git tool input: {0}")]
    InvalidInput(String),

    #[error("path is ignored: {0}")]
    IgnoredPath(String),

    #[error("path has unresolved index conflicts: {0}")]
    Conflict(String),

    #[error("unsupported git operation: {0}")]
    Unsupported(String),

    #[error("git diff exceeded its safe work limit: {0}")]
    DiffLimit(String),

    #[error("git operation cancelled")]
    Cancelled,

    #[error("cannot commit while repository operation is in progress: {0}")]
    RepositoryState(String),

    #[error("nothing staged to commit")]
    EmptyCommit,
}

impl GitError {
    /// The request is at fault (a bad ref or path, nothing staged), not git:
    /// asking differently fixes it.
    pub fn is_caller_error(&self) -> bool {
        !matches!(self, Self::Operation(_) | Self::Cancelled)
    }
}
