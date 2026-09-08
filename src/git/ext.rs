use std::path::Component;
use std::path::Path;

use crate::git::error::GitError;

pub(crate) trait GitResultExt<T> {
    fn git_op(self) -> Result<T, GitError>;
}

impl<T, E: std::fmt::Display> GitResultExt<T> for Result<T, E> {
    fn git_op(self) -> Result<T, GitError> {
        self.map_err(|e| GitError::Operation(e.to_string()))
    }
}

pub(crate) struct RepoRelativePathMessages {
    pub empty: &'static str,
    pub nul: &'static str,
    pub root: &'static str,
    pub git_dir: &'static str,
}

pub(crate) fn normalize_repo_relative_path(
    raw: &str,
    messages: RepoRelativePathMessages,
) -> Result<String, GitError> {
    if raw.trim().is_empty() {
        return Err(GitError::InvalidInput(messages.empty.to_string()));
    }
    if raw.as_bytes().contains(&0) {
        return Err(GitError::InvalidInput(messages.nul.to_string()));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(GitError::InvalidInput(format!(
            "path must be repository-relative and may not escape the repository: {raw}"
        )));
    }

    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(component) => {
                components.push(component.to_str().ok_or_else(|| {
                    GitError::InvalidInput(format!("path is not valid UTF-8: {raw}"))
                })?);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(GitError::InvalidInput(format!(
                    "path must be repository-relative and may not escape the repository: {raw}"
                )));
            }
        }
    }

    if components.is_empty() {
        return Err(GitError::InvalidInput(messages.root.to_string()));
    }
    if components[0].eq_ignore_ascii_case(".git") {
        return Err(GitError::InvalidInput(messages.git_dir.to_string()));
    }
    Ok(components.join("/"))
}
