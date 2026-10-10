use std::path::Path;
use std::sync::Arc;

use gix::bstr::ByteSlice;
use serde::Deserialize;
use serde::Serialize;

use crate::git::error::GitError;
use crate::git::ext::GitResultExt;
use crate::git::open_repo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    /// Shared by every line one commit owns.
    pub sha: Arc<str>,
    pub author: Arc<str>,
    pub line_no: usize,
    pub content: String,
}

pub fn blame(
    cwd: &Path,
    file_path: &str,
    lines: Option<(usize, usize)>,
) -> Result<Vec<BlameLine>, GitError> {
    let repo = open_repo(cwd)?;

    let head = repo.head_id().git_op()?;

    let head_obj = head.object().git_op()?;
    let head_commit = head_obj.try_into_commit().git_op()?;
    let author: Arc<str> = head_commit.author().git_op()?.name.to_str_lossy().into();
    let commit = head_commit.tree().git_op()?;

    let entry = commit
        .lookup_entry_by_path(file_path)
        .git_op()?
        .ok_or_else(|| GitError::PathNotFound(file_path.to_string()))?;

    let blob = entry.object().git_op()?;

    let content = std::str::from_utf8(&blob.data)
        .map_err(|e| GitError::Operation(format!("binary file: {e:#}")))?;

    // Without full blame traversal (which gix doesn't yet expose as a
    // simple API), we return the file content attributed to HEAD.
    // This is a placeholder until gix gains a blame API.
    let mut result = Vec::new();
    let sha: Arc<str> = head.to_hex_with_len(8).to_string().into();
    let (start, end) = lines.unwrap_or((1, usize::MAX));

    for (line_no, line) in (1..).zip(content.lines()).skip(start.saturating_sub(1)) {
        if line_no > end {
            break;
        }
        result.push(BlameLine {
            sha: Arc::clone(&sha),
            author: Arc::clone(&author), // Placeholder until full blame traversal exists.
            line_no,
            content: line.to_owned(),
        });
    }

    Ok(result)
}
