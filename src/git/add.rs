use std::path::Path;

use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::git::error::GitError;
use crate::git::ext::RepoRelativePathMessages;
use crate::git::ext::normalize_repo_relative_path;
use crate::git::open_repo;

#[derive(Debug, Clone, Serialize)]
pub struct AddResult {
    pub staged: Vec<String>,
    pub removed: Vec<String>,
}

pub fn add(cwd: &Path, paths: &[String]) -> Result<AddResult, GitError> {
    if paths.is_empty() {
        return Err(GitError::InvalidInput(
            "at least one repository-relative file path is required".to_string(),
        ));
    }

    let repo = open_repo(cwd)?;
    if repo.is_bare() {
        return Err(GitError::Unsupported("git_add requires a non-bare repository".to_string()));
    }

    let mut index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|e| GitError::Operation(format!("{e:#}")))?
        .into_owned();
    let (mut pipeline, _) =
        repo.filter_pipeline(None).map_err(|e| GitError::Operation(format!("{e:#}")))?;
    let mut excludes = repo
        .excludes(
            &index,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        )
        .map_err(|e| GitError::Operation(format!("{e:#}")))?;

    let mut normalized_paths: Vec<String> = Vec::with_capacity(paths.len());
    for path in paths {
        let normalized = normalize_explicit_path(path)?;
        if !normalized_paths.contains(&normalized) {
            normalized_paths.push(normalized);
        }
    }

    let unconflicted = gix::index::entry::Flags::from_stage(gix::index::entry::Stage::Unconflicted);
    let mut staged = Vec::new();
    let mut removed = Vec::new();
    // New entries, pushed after the loop and sorted once: pushing one unsorts
    // the index the later lookups binary-search, and each sort allocates.
    let mut new_entries = Vec::new();

    for display_path in normalized_paths {
        let path = display_path.as_bytes().as_bstr();
        let tracked = match tracked_entries(&index, path) {
            Ok(tracked) => tracked,
            Err(Unstageable::Conflict) => return Err(GitError::Conflict(display_path)),
            Err(Unstageable::Submodule) => {
                return Err(GitError::Unsupported(format!(
                    "submodule staging is not supported: {display_path}"
                )));
            }
        };

        let worktree_path =
            repo.workdir_path(path).ok_or_else(|| GitError::PathNotFound(display_path.clone()))?;
        let metadata = match gix::index::fs::Metadata::from_path_no_follow(&worktree_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if !tracked {
                    return Err(GitError::PathNotFound(display_path));
                }
                index.remove_entries(|_, entry_path, _| entry_path == path);
                removed.push(display_path);
                continue;
            }
            Err(err) => return Err(GitError::Operation(format!("{err:#}"))),
        };

        let mode = worktree_mode(&metadata, &display_path)?;

        if !tracked
            && excludes
                .at_entry(path, Some(mode))
                .map_err(|e| GitError::Operation(format!("{e:#}")))?
                .is_excluded()
        {
            return Err(GitError::IgnoredPath(display_path));
        }

        let (id, kind) = worktree_blob(&mut pipeline, &index, path, &display_path)?;
        let stat = gix::index::entry::Stat::from_fs(&metadata)
            .map_err(|e| GitError::Operation(format!("{e:#}")))?;
        if let Some(entry) =
            index.entry_mut_by_path_and_stage(path, gix::index::entry::Stage::Unconflicted)
        {
            entry.stat = stat;
            entry.id = id;
            entry.flags = unconflicted;
            entry.mode = kind.into();
        } else {
            new_entries.push((stat, id, kind, staged.len()));
        }
        staged.push(display_path);
    }

    if !new_entries.is_empty() {
        for (stat, id, kind, at) in new_entries {
            index.dangerously_push_entry(
                stat,
                id,
                unconflicted,
                kind.into(),
                staged[at].as_bytes().as_bstr(),
            );
        }
        index.sort_entries();
    }

    index.remove_tree();
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| GitError::Operation(format!("{e:#}")))?;

    Ok(AddResult { staged, removed })
}

/// Why a tracked path cannot be staged.
enum Unstageable {
    Conflict,
    Submodule,
}

/// Whether `path` is tracked, refusing conflicted entries and submodules.
fn tracked_entries(index: &gix::index::File, path: &gix::bstr::BStr) -> Result<bool, Unstageable> {
    let Some(range) = index.entry_range(path) else {
        return Ok(false);
    };
    let entries = &index.entries()[range];
    if entries.iter().any(|entry| entry.stage() != gix::index::entry::Stage::Unconflicted) {
        return Err(Unstageable::Conflict);
    }
    if entries.iter().any(|entry| entry.mode == gix::index::entry::Mode::COMMIT) {
        return Err(Unstageable::Submodule);
    }
    Ok(true)
}

/// The worktree file at `path` written as a blob, through the filters.
fn worktree_blob(
    pipeline: &mut gix::filter::Pipeline<'_>,
    index: &gix::index::File,
    path: &gix::bstr::BStr,
    display_path: &str,
) -> Result<(gix::ObjectId, gix::objs::tree::EntryKind), GitError> {
    let Some((id, kind, _)) = pipeline
        .worktree_file_to_object(path, index)
        .map_err(|e| GitError::Operation(format!("{e:#}")))?
    else {
        return Err(GitError::Unsupported(format!(
            "unable to stage worktree entry: {display_path}"
        )));
    };
    if kind == gix::objs::tree::EntryKind::Commit {
        return Err(GitError::Unsupported(format!(
            "submodule staging is not supported: {display_path}"
        )));
    }
    Ok((id, kind))
}

/// The index mode of a worktree file or symlink; anything else is refused.
fn worktree_mode(
    metadata: &gix::index::fs::Metadata,
    display_path: &str,
) -> Result<gix::index::entry::Mode, GitError> {
    if metadata.is_dir() {
        return Err(GitError::Unsupported(format!(
            "directory staging is not supported; pass files explicitly: {display_path}"
        )));
    }
    if !metadata.is_file() && !metadata.is_symlink() {
        return Err(GitError::Unsupported(format!(
            "unsupported worktree entry type: {display_path}"
        )));
    }
    Ok(if metadata.is_symlink() {
        gix::index::entry::Mode::SYMLINK
    } else if metadata.is_executable() {
        gix::index::entry::Mode::FILE_EXECUTABLE
    } else {
        gix::index::entry::Mode::FILE
    })
}

fn normalize_explicit_path(raw: &str) -> Result<String, GitError> {
    normalize_repo_relative_path(
        raw,
        RepoRelativePathMessages {
            empty: "git_add paths must not be empty",
            nul: "git_add paths must not contain NUL bytes",
            root: "repository root cannot be staged; pass files explicitly",
            git_dir: "paths inside the Git directory cannot be staged",
        },
    )
}
