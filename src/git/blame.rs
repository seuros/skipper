use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::Path;
use std::sync::Arc;

use gix::blame::BlameRanges;
use gix::bstr::ByteSlice;
use serde::Deserialize;
use serde::Serialize;

use crate::git::error::GitError;
use crate::git::ext::{GitResultExt, RepoRelativePathMessages, normalize_repo_relative_path};
use crate::git::open_repo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    /// The commit that last changed the line, short; shared by every line it
    /// owns.
    pub sha: Arc<str>,
    pub author: Arc<str>,
    pub line_no: usize,
    pub content: String,
}

/// Each line of `file_path` at HEAD with the commit that last changed it,
/// following renames. `lines` (1-based, inclusive) narrows what gets blamed,
/// not just what is returned; lines past the end are left out.
pub fn blame(
    cwd: &Path,
    file_path: &str,
    lines: Option<(usize, usize)>,
) -> Result<Vec<BlameLine>, GitError> {
    let path = normalize_repo_relative_path(
        file_path,
        RepoRelativePathMessages {
            empty: "file_path must not be empty",
            nul: "file_path must not contain NUL bytes",
            root: "repository root cannot be blamed; pass a file path",
            git_dir: "paths inside the Git directory cannot be blamed",
        },
    )?;
    let ranges = match lines {
        None => BlameRanges::default(),
        Some((start, end)) => {
            let line = |n: usize| {
                u32::try_from(n).map_err(|_| GitError::InvalidInput(format!("no line {n}")))
            };
            BlameRanges::from_one_based_inclusive_range(line(start)?..=line(end)?)
                .map_err(|e| GitError::InvalidInput(format!("line range {start}..={end}: {e}")))?
        }
    };

    let repo = open_repo(cwd)?;
    let head = repo.head_id().git_op()?.detach();
    let entry = repo
        .find_commit(head)
        .git_op()?
        .tree()
        .git_op()?
        .lookup_entry_by_path(path.as_str())
        .git_op()?
        .ok_or_else(|| GitError::PathNotFound(path.clone()))?;
    if !entry.mode().is_blob_or_symlink() {
        return Err(GitError::Unsupported(format!("not a file: {path}")));
    }

    let options = gix::repository::blame_file::Options {
        ranges,
        rewrites: Some(gix::diff::Rewrites::default()),
        ..Default::default()
    };
    let outcome = repo.blame_file(path.as_bytes().as_bstr(), head, options).git_op()?;
    let content = std::str::from_utf8(&outcome.blob)
        .map_err(|_| GitError::Unsupported(format!("binary file: {path}")))?;

    // One lookup per commit; its lines share the sha and author.
    let mut commits: HashMap<gix::ObjectId, (Arc<str>, Arc<str>)> = HashMap::new();
    let mut entries = outcome.entries;
    entries.sort_by_key(|entry| entry.start_in_blamed_file);
    let mut file_lines = content.lines().enumerate();
    let mut result = Vec::new();
    for entry in entries {
        let (sha, author) = match commits.entry(entry.commit_id) {
            Entry::Occupied(known) => known.get().clone(),
            Entry::Vacant(slot) => {
                let commit = repo.find_commit(entry.commit_id).git_op()?;
                let author: Arc<str> = commit.author().git_op()?.name.to_str_lossy().into();
                let sha: Arc<str> = entry.commit_id.to_hex_with_len(8).to_string().into();
                slot.insert((Arc::clone(&sha), Arc::clone(&author)));
                (sha, author)
            }
        };
        let range = entry.range_in_blamed_file();
        let hunk = file_lines.by_ref().skip_while(|(i, _)| *i < range.start).take(range.len());
        for (i, line) in hunk {
            result.push(BlameLine {
                sha: Arc::clone(&sha),
                author: Arc::clone(&author),
                line_no: i + 1,
                content: line.to_owned(),
            });
        }
    }

    Ok(result)
}
