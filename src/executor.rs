use crate::error::{CliError, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == 0
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self, cli: &str) -> Result<T> {
        serde_json::from_str(&self.stdout).map_err(|e| CliError::json(cli, e))
    }
}

pub async fn execute(cli: &str, args: &[&str], timeout_duration: Duration) -> Result<Output> {
    let mut cmd = Command::new(cli);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    tracing::debug!(cli = cli, args = ?args, "executing command");

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::not_installed(cli)
        } else {
            CliError::io(cli, e)
        }
    })?;

    let output = timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| CliError::timeout(cli, timeout_duration))?
        .map_err(|e| CliError::io(cli, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    tracing::debug!(cli = cli, code = code, "command completed");

    Ok(Output { stdout, stderr, code })
}

pub async fn execute_default(cli: &str, args: &[&str]) -> Result<Output> {
    execute(cli, args, DEFAULT_TIMEOUT).await
}

pub async fn execute_success(
    cli: &str,
    args: &[&str],
    timeout_duration: Duration,
) -> Result<Output> {
    let output = execute(cli, args, timeout_duration).await?;

    if !output.success() {
        return Err(CliError::execution_failed(cli, output.code, &output.stderr));
    }

    Ok(output)
}

pub async fn is_installed(cli: &str) -> bool {
    execute(cli, &["--version"], Duration::from_secs(5)).await.is_ok()
}

pub async fn get_version_output(cli: &str) -> Result<String> {
    let output = execute(cli, &["--version"], Duration::from_secs(5)).await?;

    if output.success() {
        Ok(output.stdout)
    } else if output.stderr.to_lowercase().contains("version") {
        Ok(output.stderr)
    } else {
        Err(CliError::execution_failed(cli, output.code, &output.stderr))
    }
}

#[cfg(test)]
mod tests;
