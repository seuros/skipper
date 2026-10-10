use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{cli} is not installed or not in PATH")]
    NotInstalled { cli: String },

    #[error("{cli} requires authentication - run `{cli} auth login`")]
    AuthRequired { cli: String },

    #[error("{cli} command failed (exit code {code}): {stderr}")]
    ExecutionFailed { cli: String, code: i32, stderr: String },

    #[error("{cli} output parse error: {reason}")]
    ParseError { cli: String, output: String, reason: String },

    #[error("{cli} command timed out after {duration:?}")]
    Timeout { cli: String, duration: Duration },

    #[error("{cli} IO error: {source}")]
    Io {
        cli: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{cli} JSON error: {source}")]
    Json {
        cli: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("{cli} does not support {operation}")]
    Unsupported { cli: String, operation: String },

    /// No repo, PR or run fits the request (no matching remote, no PR for
    /// the branch); the message says what to change.
    #[error("{0}")]
    NoTarget(String),
}

impl CliError {
    pub fn not_installed(cli: impl Into<String>) -> Self {
        Self::NotInstalled { cli: cli.into() }
    }

    pub fn auth_required(cli: impl Into<String>) -> Self {
        Self::AuthRequired { cli: cli.into() }
    }

    pub fn execution_failed(cli: impl Into<String>, code: i32, stderr: impl Into<String>) -> Self {
        Self::ExecutionFailed { cli: cli.into(), code, stderr: stderr.into() }
    }

    pub fn parse_error(
        cli: impl Into<String>,
        output: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::ParseError { cli: cli.into(), output: output.into(), reason: reason.into() }
    }

    pub fn timeout(cli: impl Into<String>, duration: Duration) -> Self {
        Self::Timeout { cli: cli.into(), duration }
    }

    pub fn io(cli: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io { cli: cli.into(), source }
    }

    pub fn json(cli: impl Into<String>, source: serde_json::Error) -> Self {
        Self::Json { cli: cli.into(), source }
    }

    pub fn unsupported(cli: impl Into<String>, operation: impl Into<String>) -> Self {
        Self::Unsupported { cli: cli.into(), operation: operation.into() }
    }

    pub fn no_target(message: impl Into<String>) -> Self {
        Self::NoTarget(message.into())
    }
}

pub type Result<T> = std::result::Result<T, CliError>;
