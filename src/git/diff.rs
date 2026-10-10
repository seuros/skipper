use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gix::bstr::{ByteSlice, ByteVec};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use similar::ChangeTag;
use similar::TextDiff;

use crate::git::error::GitError;
use crate::git::ext::GitResultExt;
use crate::git::open_repo;

const MAX_CHANGED_FILES: usize = 20_000;
const MAX_BLOB_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOTAL_CONTENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEXT_LINES: usize = 200_000;
const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_WHITESPACE_ERRORS: usize = 10_000;
const TEXT_DIFF_TIMEOUT: Duration = Duration::from_millis(200);
/// Worktree files are read this much at a time, so a cancel lands between.
const READ_CHUNK: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    Worktree,
    Staged,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiffFormat {
    /// Return unified patches grouped by file.
    Patch,
    /// Return per-file and aggregate line statistics.
    Stat,
    /// Return repository-relative changed paths.
    NameOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub status: DiffStatus,
    pub binary: bool,
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
    pub patch: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffSummary {
    pub files_changed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insertions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_files: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhitespaceError {
    /// Shared by every error in one file.
    pub path: Arc<str>,
    pub line: usize,
    pub kind: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    /// Changed paths, listed for `name_only` alone; the other formats carry
    /// them in `files`.
    pub paths: Vec<String>,
    pub files: Vec<DiffFile>,
    pub summary: DiffSummary,
    pub whitespace_errors: Vec<WhitespaceError>,
}

pub(crate) fn diff_report_with_cancel(
    cwd: &Path,
    scope: DiffScope,
    format: DiffFormat,
    base: Option<&str>,
    paths: Option<&[&str]>,
    check_whitespace: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<DiffReport, GitError> {
    if scope == DiffScope::Worktree && base.is_some() {
        return Err(GitError::InvalidInput(
            "base cannot be used with worktree scope; worktree compares the index to the working tree"
                .to_string(),
        ));
    }

    check_cancelled(cancel)?;
    let repo = open_repo(cwd)?;
    let root = repo
        .workdir()
        .ok_or_else(|| GitError::Operation("repository has no worktree".to_string()))?;
    // Only read: the shared index is borrowed, not cloned.
    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|e| GitError::Operation(format!("{e:#}")))?;

    let base_tree = match scope {
        DiffScope::Worktree => None,
        DiffScope::Staged | DiffScope::All => Some(resolve_base_tree(&repo, base)?),
    };
    let (changed_paths, paths_requiring_confirmation) =
        changed_paths(&repo, &index, base_tree.as_ref(), scope, paths, cancel)?;

    let needs_details = format != DiffFormat::NameOnly || check_whitespace;
    let list_paths = format == DiffFormat::NameOnly;
    let mut files = Vec::new();
    let mut files_changed = 0usize;
    let mut confirmed_paths = Vec::new();
    let mut whitespace_errors = Vec::new();
    let mut total_content_bytes = 0usize;
    let mut total_patch_bytes = 0usize;
    for path in changed_paths {
        check_cancelled(cancel)?;
        let needs_content = needs_details || paths_requiring_confirmation.contains(path.as_str());
        if !needs_content {
            files_changed += 1;
            confirmed_paths.push(path);
            continue;
        }

        let old_content = match &base_tree {
            None => index_blob_content(&repo, &index, &path)?,
            Some(base_tree) => tree_blob_content(&repo, base_tree, &path)?,
        };
        let new_content = match scope {
            DiffScope::Staged => index_blob_content(&repo, &index, &path)?,
            DiffScope::Worktree | DiffScope::All => worktree_blob_content(root, &path, cancel)?,
        };

        if old_content == new_content {
            continue;
        }

        files_changed += 1;
        if !needs_details {
            confirmed_paths.push(path);
            continue;
        }
        if list_paths {
            confirmed_paths.push(path.clone());
        }

        let content_bytes = old_content.as_deref().map_or(0, <[u8]>::len)
            + new_content.as_deref().map_or(0, <[u8]>::len);
        total_content_bytes = total_content_bytes.saturating_add(content_bytes);
        if total_content_bytes > MAX_TOTAL_CONTENT_BYTES {
            return Err(GitError::DiffLimit(format!(
                "content exceeds {MAX_TOTAL_CONTENT_BYTES} bytes in total; narrow paths or use format=name_only"
            )));
        }

        let (file, mut file_errors) = build_diff_file(
            path,
            old_content.as_deref(),
            new_content.as_deref(),
            format,
            check_whitespace,
        )?;
        total_patch_bytes = total_patch_bytes.saturating_add(file.patch.len());
        if total_patch_bytes > MAX_PATCH_BYTES {
            return Err(GitError::DiffLimit(format!(
                "patch output exceeds {MAX_PATCH_BYTES} bytes; narrow paths or use format=stat/name_only"
            )));
        }
        if whitespace_errors.len().saturating_add(file_errors.len()) > MAX_WHITESPACE_ERRORS {
            return Err(GitError::DiffLimit(format!(
                "whitespace check produced more than {MAX_WHITESPACE_ERRORS} errors in total; narrow paths"
            )));
        }
        check_cancelled(cancel)?;
        files.push(file);
        whitespace_errors.append(&mut file_errors);
    }

    let summary = DiffSummary {
        files_changed,
        insertions: needs_details.then(|| files.iter().filter_map(|file| file.additions).sum()),
        deletions: needs_details.then(|| files.iter().filter_map(|file| file.deletions).sum()),
        binary_files: needs_details.then(|| files.iter().filter(|file| file.binary).count()),
    };

    Ok(DiffReport { paths: confirmed_paths, files, summary, whitespace_errors })
}

/// The paths to diff, and those changed both staged and in the worktree:
/// their content needs comparing, since the worktree may undo what was
/// staged. The two sides merge by move.
fn changed_paths(
    repo: &gix::Repository,
    index: &gix::index::File,
    base_tree: Option<&gix::Tree<'_>>,
    scope: DiffScope,
    paths: Option<&[&str]>,
    cancel: &Arc<AtomicBool>,
) -> Result<(BTreeSet<String>, BTreeSet<String>), GitError> {
    let mut staged_paths = BTreeSet::new();
    if let Some(base_tree) = base_tree {
        collect_tree_index_paths(repo, index, base_tree, paths, &mut staged_paths, cancel)?;
    }
    let mut unstaged_paths = BTreeSet::new();
    if scope != DiffScope::Staged {
        collect_worktree_paths(repo, paths, &mut unstaged_paths, cancel)?;
    }

    let (changed, both) = match scope {
        DiffScope::Worktree => (unstaged_paths, BTreeSet::new()),
        DiffScope::Staged => (staged_paths, BTreeSet::new()),
        DiffScope::All => {
            let mut both = BTreeSet::new();
            let mut changed = staged_paths;
            for path in unstaged_paths {
                if changed.contains(&path) {
                    both.insert(path);
                } else {
                    changed.insert(path);
                }
            }
            (changed, both)
        }
    };
    ensure_changed_path_limit(changed.len())?;
    Ok((changed, both))
}

fn resolve_base_tree<'repo>(
    repo: &'repo gix::Repository,
    base: Option<&str>,
) -> Result<gix::Tree<'repo>, GitError> {
    let base_spec = base.unwrap_or("HEAD");
    if base_spec == "HEAD" {
        return match repo.head_id() {
            Ok(id) => id.object().git_op()?.peel_to_tree().git_op(),
            Err(_) => Ok(repo.empty_tree()),
        };
    }

    crate::git::rev_parse(repo, base_spec)?.object().git_op()?.peel_to_tree().git_op()
}

fn collect_tree_index_paths(
    repo: &gix::Repository,
    index: &gix::index::File,
    tree: &gix::Tree<'_>,
    paths: Option<&[&str]>,
    changed_paths: &mut BTreeSet<String>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), GitError> {
    let mut limit_exceeded = false;
    repo.tree_index_status(
        tree.id.as_ref(),
        index,
        None,
        gix::status::tree_index::TrackRenames::Disabled,
        |change, _, _| {
            use gix::diff::index::ChangeRef;

            let (ChangeRef::Addition { location, .. }
            | ChangeRef::Deletion { location, .. }
            | ChangeRef::Modification { location, .. }
            | ChangeRef::Rewrite { location, .. }) = change;
            // Owned only once the filter keeps it.
            let path = location.to_str_lossy();
            if matches_filter(&path, paths) {
                changed_paths.insert(path.into_owned());
            }
            limit_exceeded = changed_paths.len() > MAX_CHANGED_FILES;
            let stop = limit_exceeded || cancel.load(Ordering::Acquire);
            Ok(if stop { ControlFlow::Break(()) } else { ControlFlow::Continue(()) })
        },
    )
    .git_op()?;
    check_cancelled(cancel)?;
    if limit_exceeded {
        return Err(changed_path_limit_error());
    }
    Ok(())
}

fn collect_worktree_paths(
    repo: &gix::Repository,
    paths: Option<&[&str]>,
    changed_paths: &mut BTreeSet<String>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), GitError> {
    use gix::status::index_worktree::Item;

    let status_iter = repo
        .status(gix::progress::Discard)
        .git_op()?
        .untracked_files(gix::status::UntrackedFiles::None)
        .index_worktree_submodules(None)
        .should_interrupt_owned(Arc::clone(cancel))
        .into_index_worktree_iter(Vec::<gix::bstr::BString>::new())
        .git_op()?;

    for item in status_iter {
        check_cancelled(cancel)?;
        let item = item.git_op()?;
        if let Item::Modification { rela_path, .. } = item {
            let path = Vec::from(rela_path).into_string_lossy();
            if matches_filter(&path, paths) {
                changed_paths.insert(path);
                ensure_changed_path_limit(changed_paths.len())?;
            }
        }
    }
    check_cancelled(cancel)
}

fn tree_blob_content(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    path: &str,
) -> Result<Option<Vec<u8>>, GitError> {
    let Some(entry) = tree.lookup_entry_by_path(path).git_op()? else {
        return Ok(None);
    };
    object_blob_content(repo, entry.id().detach(), path)
}

fn index_blob_content(
    repo: &gix::Repository,
    index: &gix::index::File,
    path: &str,
) -> Result<Option<Vec<u8>>, GitError> {
    let Some(range) = index.entry_range(path.as_bytes().as_bstr()) else {
        return Ok(None);
    };
    let entry = index.entries()[range]
        .iter()
        .find(|entry| entry.stage() == gix::index::entry::Stage::Unconflicted)
        .ok_or_else(|| GitError::Conflict(path.to_string()))?;
    object_blob_content(repo, entry.id, path)
}

fn object_blob_content(
    repo: &gix::Repository,
    id: gix::ObjectId,
    path: &str,
) -> Result<Option<Vec<u8>>, GitError> {
    ensure_object_size(repo, id, path)?;
    Ok(Some(repo.find_object(id).git_op()?.detach().data))
}

fn worktree_blob_content(
    root: &Path,
    path: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<Vec<u8>>, GitError> {
    let full_path = root.join(path);
    let metadata = match fs::symlink_metadata(&full_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(GitError::Operation(format!(
                "failed to inspect worktree file {path}: {err}"
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(&full_path).map_err(|err| {
            GitError::Operation(format!("failed to read worktree symlink {path}: {err}"))
        })?;
        // Git stores a symlink as its target's bytes; compare those, not a
        // lossy UTF-8 rendering of them.
        let content = target.into_os_string().into_encoded_bytes();
        ensure_blob_size(path, content.len() as u64)?;
        return Ok(Some(content));
    }
    if !metadata.is_file() {
        return Ok(None);
    }
    ensure_blob_size(path, metadata.len())?;

    let mut file = fs::File::open(&full_path).map_err(|err| {
        GitError::Operation(format!("failed to open worktree file {path}: {err}"))
    })?;
    // Read straight into the result, a chunk at a time so a cancel lands
    // between chunks; no bounce buffer to copy out of.
    let mut content = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(MAX_BLOB_BYTES));
    loop {
        check_cancelled(cancel)?;
        let read = (&mut file).take(READ_CHUNK).read_to_end(&mut content).map_err(|err| {
            GitError::Operation(format!("failed to read worktree file {path}: {err}"))
        })?;
        if read == 0 {
            break;
        }
        ensure_blob_size(path, content.len() as u64)?;
    }
    Ok(Some(content))
}

fn build_diff_file(
    path: String,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    format: DiffFormat,
    check_whitespace: bool,
) -> Result<(DiffFile, Vec<WhitespaceError>), GitError> {
    let status = match (old.is_some(), new.is_some()) {
        (false, true) => DiffStatus::Added,
        (true, false) => DiffStatus::Deleted,
        (true, true) => DiffStatus::Modified,
        (false, false) => unreachable!("unchanged missing file was filtered"),
    };
    let binary = old.is_some_and(is_binary) || new.is_some_and(is_binary);

    let render_patch = format == DiffFormat::Patch;
    // Patch headers, built only for a patch.
    let label = |side: &str, present: bool| -> Cow<'static, str> {
        if present { Cow::Owned(format!("{side}/{path}")) } else { Cow::Borrowed("/dev/null") }
    };
    let (from, to) = if render_patch {
        (label("a", old.is_some()), label("b", new.is_some()))
    } else {
        (Cow::Borrowed(""), Cow::Borrowed(""))
    };
    let mut rendered =
        if render_patch { format!("diff --git a/{path} b/{path}\n") } else { String::new() };
    if binary {
        if render_patch {
            writeln!(rendered, "Binary files {from} and {to} differ")
                .expect("writing to a String cannot fail");
        }
        return Ok((
            DiffFile { path, status, binary, additions: None, deletions: None, patch: rendered },
            Vec::new(),
        ));
    }

    let old_text = String::from_utf8_lossy(old.unwrap_or_default());
    let new_text = String::from_utf8_lossy(new.unwrap_or_default());
    let line_count = old_text.lines().count().saturating_add(new_text.lines().count());
    if line_count > MAX_TEXT_LINES {
        return Err(GitError::DiffLimit(format!(
            "{path} contains {line_count} lines across both sides (limit {MAX_TEXT_LINES}); use format=name_only"
        )));
    }
    let text_diff = TextDiff::configure()
        .timeout(TEXT_DIFF_TIMEOUT)
        .diff_lines(old_text.as_ref(), new_text.as_ref());
    let mut additions = 0;
    let mut deletions = 0;
    for change in text_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    if render_patch {
        write!(rendered, "{}", text_diff.unified_diff().context_radius(3).header(&from, &to))
            .expect("writing to a String cannot fail");
    }
    let whitespace_errors =
        if check_whitespace { collect_whitespace_errors(&path, &text_diff)? } else { Vec::new() };

    Ok((
        DiffFile {
            path,
            status,
            binary,
            additions: Some(additions),
            deletions: Some(deletions),
            patch: rendered,
        },
        whitespace_errors,
    ))
}

fn collect_whitespace_errors(
    path: &str,
    diff: &TextDiff<'_, '_, str>,
) -> Result<Vec<WhitespaceError>, GitError> {
    let mut errors = Vec::new();
    let mut new_line = 0;
    let shared_path: Arc<str> = Arc::from(path);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => continue,
            ChangeTag::Equal => {
                new_line += 1;
                continue;
            }
            ChangeTag::Insert => new_line += 1,
        }

        let value = change.value();
        let line = value.strip_suffix('\n').unwrap_or(value);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut report = |kind, message| {
            if errors.len() >= MAX_WHITESPACE_ERRORS {
                return Err(GitError::DiffLimit(format!(
                    "whitespace check produced more than {MAX_WHITESPACE_ERRORS} errors; narrow paths"
                )));
            }
            errors.push(WhitespaceError {
                path: Arc::clone(&shared_path),
                line: new_line,
                kind,
                message,
            });
            Ok(())
        };
        if line.ends_with([' ', '\t']) {
            report("trailing_whitespace", "new line has trailing whitespace")?;
        }
        if has_space_before_tab_in_indent(line) {
            report("space_before_tab", "new line has a space before a tab in its indentation")?;
        }
        if is_conflict_marker(line) {
            report("conflict_marker", "new line introduces a conflict marker")?;
        }
    }
    Ok(errors)
}

fn ensure_object_size(
    repo: &gix::Repository,
    id: gix::ObjectId,
    path: &str,
) -> Result<(), GitError> {
    let header = repo.find_header(id).git_op()?;
    ensure_blob_size(path, header.size())
}

fn ensure_blob_size(path: &str, size: u64) -> Result<(), GitError> {
    if size > MAX_BLOB_BYTES as u64 {
        return Err(GitError::DiffLimit(format!(
            "{path} is {size} bytes (per-file limit {MAX_BLOB_BYTES}); use format=name_only when possible or narrow paths"
        )));
    }
    Ok(())
}

fn ensure_changed_path_limit(count: usize) -> Result<(), GitError> {
    if count > MAX_CHANGED_FILES {
        return Err(changed_path_limit_error());
    }
    Ok(())
}

fn changed_path_limit_error() -> GitError {
    GitError::DiffLimit(format!("more than {MAX_CHANGED_FILES} changed files; narrow paths"))
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), GitError> {
    if cancel.load(Ordering::Acquire) { Err(GitError::Cancelled) } else { Ok(()) }
}

fn has_space_before_tab_in_indent(line: &str) -> bool {
    let mut saw_space = false;
    for byte in line.bytes() {
        match byte {
            b' ' => saw_space = true,
            b'\t' if saw_space => return true,
            b'\t' => {}
            _ => break,
        }
    }
    false
}

fn is_conflict_marker(line: &str) -> bool {
    ["<<<<<<<", "=======", ">>>>>>>"].iter().any(|marker| line.starts_with(marker))
}

pub(crate) fn is_binary(content: &[u8]) -> bool {
    content[..content.len().min(8192)].contains(&0)
}

fn matches_filter(path: &str, paths: Option<&[&str]>) -> bool {
    match paths {
        None => true,
        Some(filters) => filters.iter().any(|filter| {
            let filter = filter.trim_end_matches('/');
            path == filter
                || path.strip_prefix(filter).is_some_and(|suffix| suffix.starts_with('/'))
        }),
    }
}
