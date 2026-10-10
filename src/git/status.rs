use std::borrow::Cow;
use std::ops::ControlFlow;
use std::path::Path;

use gix::bstr::{BStr, BString, ByteSlice, ByteVec};
use serde::Serialize;

use crate::git::error::GitError;
use crate::git::ext::GitResultExt;
use crate::git::open_repo;

#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusInfo {
    pub staged: Vec<FileStatus>,
    pub unstaged: Vec<FileStatus>,
    pub untracked: Vec<FileStatus>,
}

/// How many paths [`StatusInfo`] would list, without building the lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    Staged,
    Unstaged,
    Untracked,
}

/// Which comparisons [`walk`] runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Index vs. worktree, then HEAD vs. index.
    All,
    /// HEAD vs. index alone.
    Staged,
}

pub fn collect(cwd: &Path) -> Result<StatusInfo, GitError> {
    let mut info = StatusInfo { staged: Vec::new(), unstaged: Vec::new(), untracked: Vec::new() };
    walk(cwd, Scope::All, |kind, path| {
        let list = match kind {
            Kind::Staged => &mut info.staged,
            Kind::Unstaged => &mut info.unstaged,
            Kind::Untracked => &mut info.untracked,
        };
        // Worktree paths arrive owned and keep their buffer.
        let path = match path {
            Cow::Owned(path) => Vec::from(path).into_string_lossy(),
            Cow::Borrowed(path) => path.to_str_lossy().into_owned(),
        };
        list.push(FileStatus { path });
        ControlFlow::Continue(())
    })?;
    Ok(info)
}

pub fn counts(cwd: &Path) -> Result<StatusCounts, GitError> {
    let mut counts = StatusCounts::default();
    walk(cwd, Scope::All, |kind, _| {
        match kind {
            Kind::Staged => counts.staged += 1,
            Kind::Unstaged => counts.unstaged += 1,
            Kind::Untracked => counts.untracked += 1,
        }
        ControlFlow::Continue(())
    })?;
    Ok(counts)
}

/// No staged, modified or untracked path; stops at the first one.
pub fn is_clean(cwd: &Path) -> Result<bool, GitError> {
    any(cwd, Scope::All).map(|found| !found)
}

/// Something is staged; stops at the first staged path.
pub fn has_staged(cwd: &Path) -> Result<bool, GitError> {
    any(cwd, Scope::Staged)
}

fn any(cwd: &Path, scope: Scope) -> Result<bool, GitError> {
    let mut found = false;
    walk(cwd, scope, |_, _| {
        found = true;
        ControlFlow::Break(())
    })?;
    Ok(found)
}

/// Hands every changed path to `visit` until it breaks: owned where gix
/// hands it over owned, borrowed otherwise.
fn walk(
    cwd: &Path,
    scope: Scope,
    mut visit: impl FnMut(Kind, Cow<'_, BStr>) -> ControlFlow<()>,
) -> Result<(), GitError> {
    use gix::diff::index::ChangeRef;
    use gix::status::index_worktree::Item;

    let repo = open_repo(cwd)?;

    if scope == Scope::All {
        let worktree = repo
            .status(gix::progress::Discard)
            .git_op()?
            .into_index_worktree_iter(Vec::<BString>::new())
            .git_op()?;
        for item in worktree {
            let flow = match item.git_op()? {
                Item::Modification { rela_path, .. } => {
                    visit(Kind::Unstaged, Cow::Owned(rela_path))
                }
                Item::DirectoryContents { entry, .. } => {
                    visit(Kind::Untracked, Cow::Owned(entry.rela_path))
                }
                Item::Rewrite { .. } => ControlFlow::Continue(()),
            };
            if flow.is_break() {
                return Ok(());
            }
        }
    }

    let head_tree_id = repo.head_tree_id().map_or_else(|_| repo.empty_tree().id, gix::Id::detach);
    let index = repo.index_or_empty().git_op()?;
    repo.tree_index_status(
        head_tree_id.as_ref(),
        &index,
        None,
        gix::status::tree_index::TrackRenames::Disabled,
        |change, _, _| {
            let (ChangeRef::Addition { location, .. }
            | ChangeRef::Deletion { location, .. }
            | ChangeRef::Modification { location, .. }
            | ChangeRef::Rewrite { location, .. }) = &change;
            Ok(visit(Kind::Staged, Cow::Borrowed(location.as_ref())))
        },
    )
    .git_op()?;
    Ok(())
}
