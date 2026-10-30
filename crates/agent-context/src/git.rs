//! Git status/diff sniffing for prompt injection.
//!
//! Contract (kernel-architecture.md §Workspace scanner):
//! `git_snapshot()` → `git status --porcelain` + `git diff --stat`,
//! truncated to ~4 KB. Implemented with `gix` (gitoxide) — pure Rust,
//! in-process, zero C linkage (Operating Principle 1).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Component;
use std::path::{Path, PathBuf};

use similar::TextDiff;
use thiserror::Error;

/// Total byte budget for the rendered snapshot block.
pub const GIT_SNAPSHOT_BUDGET: usize = 4 * 1024;

/// Legacy API's per-file input/diff limit, retained for existing callers.
const MAX_DIFF_BYTES: u64 = 1024 * 1024;
/// Scoped API bounds inputs before allocating or diffing.
const MAX_SAFE_DIFF_BYTES: u64 = 128 * 1024;
const MAX_SCAN_ENTRIES: usize = 20_000;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;

/// Cap on files per changes read — the panel is a summary, not an archive.
const MAX_CHANGED_FILES: usize = 300;

/// Only the head of a file is sniffed for NUL bytes, like git does.
const BINARY_SNIFF_LEN: usize = 8 * 1024;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("not a git repository (or any parent): {0}")]
    NotARepo(PathBuf),

    #[error("git operation failed: {0}")]
    Op(String),
}

impl From<gix::status::into_iter::Error> for GitError {
    fn from(e: gix::status::into_iter::Error) -> Self {
        GitError::Op(e.to_string())
    }
}

fn discover_repository(path: &Path) -> Result<gix::Repository, GitError> {
    use gix::discover::{upwards, Error};
    match gix::discover(path) {
        Ok(repository) => Ok(repository),
        Err(Error::Discover(
            upwards::Error::NoGitRepository { .. }
            | upwards::Error::NoGitRepositoryWithinCeiling { .. }
            | upwards::Error::NoGitRepositoryWithinFs { .. },
        )) => Err(GitError::NotARepo(path.to_path_buf())),
        Err(error) => Err(GitError::Op(error.to_string())),
    }
}

/// How a worktree file differs from its HEAD version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Untracked, or added to the index but absent from HEAD.
    Add,
    /// In HEAD and modified — staged, unstaged or both.
    Update,
    /// In HEAD but gone from the worktree.
    Delete,
}

impl ChangeKind {
    /// Lowercase name as the changes panel consumes it.
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeKind::Add => "add",
            ChangeKind::Update => "update",
            ChangeKind::Delete => "delete",
        }
    }
}

/// One uncommitted file: HEAD's blob against the worktree bytes, with the
/// line counts and unified diff the changes panel renders.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Path relative to the worktree root, as git tracks it.
    pub path: String,
    pub kind: ChangeKind,
    pub adds: u32,
    pub dels: u32,
    /// Unified diff with 3 lines of context. Empty for binary files and for
    /// anything flagged `truncated`.
    pub diff: String,
    /// A NUL byte up front or non-UTF-8 content — there is no line diff.
    pub binary: bool,
    /// The diff was dropped: oversized/unavailable content, or past [`MAX_CHANGED_FILES`].
    pub truncated: bool,
}

/// A point-in-time git summary suitable for the `{{GIT_STATUS}}` prompt
/// substitution. `status` mirrors `git status --porcelain` (one
/// `XY <path>` line per entry); `diff_stat` mirrors `git diff --stat`.
#[derive(Debug, Clone, Default)]
pub struct GitSnapshot {
    /// Repository workdir root (canonicalized when available).
    pub repo_root: PathBuf,
    /// Current branch name, or `detached:<short-id>` when detached.
    /// `None` on an unborn HEAD (fresh repo with no commits).
    pub branch: Option<String>,
    /// Porcelain-style status lines, e.g. ` M src/main.rs`, `?? new.rs`.
    pub status: Vec<String>,
    /// `git diff --stat`-style per-file `name | N +/-` lines covering
    /// staged + unstaged changes.
    pub diff_stat: Vec<String>,
    /// True when a section was cut to fit [`GIT_SNAPSHOT_BUDGET`].
    pub truncated: bool,
}

impl GitSnapshot {
    /// Total dirty entry count across the porcelain section.
    pub fn dirty_count(&self) -> usize {
        self.status.len()
    }

    /// Render as the `{{GIT_STATUS}}` prompt block. Always ≤ ~4 KB.
    pub fn to_prompt_block(&self) -> String {
        let mut out = String::with_capacity(GIT_SNAPSHOT_BUDGET);
        match &self.branch {
            Some(b) => {
                let _ = writeln!(out, "branch: {b}");
            }
            None => {
                let _ = writeln!(out, "branch: (unborn HEAD)");
            }
        }

        if self.status.is_empty() {
            let _ = writeln!(out, "status: clean");
        } else {
            let _ = writeln!(out, "status:");
            for line in &self.status {
                let _ = writeln!(out, "  {line}");
            }
        }

        if !self.diff_stat.is_empty() {
            let _ = writeln!(out, "diff --stat:");
            for line in &self.diff_stat {
                let _ = writeln!(out, "  {line}");
            }
        }

        if self.truncated {
            let _ = writeln!(out, "... [truncated]");
        }
        out
    }
}

/// Snapshot the git state of the repository containing `path`.
///
/// Returns [`GitError::NotARepo`] when `path` isn't inside a worktree — the
/// prompt assembler treats this as "no git context" and substitutes an empty
/// block, not an error.
pub fn git_snapshot(path: impl AsRef<Path>) -> Result<GitSnapshot, GitError> {
    let path = path.as_ref();
    let repo = discover_repository(path)?;
    snapshot_repo(&repo)
}

fn snapshot_repo(repo: &gix::Repository) -> Result<GitSnapshot, GitError> {
    let repo_root = repo
        .workdir()
        .map(|w| w.canonicalize().unwrap_or_else(|_| w.to_path_buf()))
        .unwrap_or_else(|| repo.path().to_path_buf());

    let branch = current_branch(repo);
    let (status, diff_stat) = collect_status_and_stat(repo)?;

    let mut snap = GitSnapshot {
        repo_root,
        branch,
        status,
        diff_stat,
        truncated: false,
    };
    enforce_budget(&mut snap);
    Ok(snap)
}

/// Branch name, or `detached:<short-id>`, or `None` when HEAD is unborn.
fn current_branch(repo: &gix::Repository) -> Option<String> {
    match repo.head_name() {
        Ok(Some(name)) => {
            let short = name.shorten();
            Some(short.to_string())
        }
        Ok(None) => repo
            .head_id()
            .ok()
            .map(|id| format!("detached:{}", &id.to_hex_with_len(7).to_string())),
        Err(_) => None,
    }
}

/// One pass over `repo.status()` producing both the porcelain lines and the
/// per-file change counts used for the `--stat` section. Untracked files
/// have no content diff, so they contribute to `status` only.
fn collect_status_and_stat(repo: &gix::Repository) -> Result<(Vec<String>, Vec<String>), GitError> {
    use gix::status::index_worktree;
    use gix::status::plumbing::index_as_worktree::{Change as WtChange, EntryStatus};
    use gix::status::Item;

    let mut status_lines: Vec<String> = Vec::new();
    // Paths aggregated for the --stat section.
    let mut stat: Vec<String> = Vec::new();

    let platform = repo
        .status(gix::progress::Discard)
        .map_err(|e| GitError::Op(e.to_string()))?
        // Porcelain default: collapsed untracked dirs → `?? dir/` entries.
        .untracked_files(gix::status::UntrackedFiles::Collapsed)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled);

    let iter = platform.into_iter(Vec::<gix::bstr::BString>::new())?;

    for item in iter {
        let item = match item {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(%e, "status iter error, skipping entry");
                continue;
            }
        };

        match item {
            Item::TreeIndex(change) => {
                use gix::diff::index::Change;
                let (code, path) = match &change {
                    Change::Addition { location, .. } => ("A", location.to_string()),
                    Change::Deletion { location, .. } => ("D", location.to_string()),
                    Change::Modification { location, .. } => ("M", location.to_string()),
                    Change::Rewrite {
                        source_location,
                        location,
                        copy,
                        ..
                    } => {
                        let c = if *copy { "C" } else { "R" };
                        (c, format!("{} -> {}", source_location, location))
                    }
                };
                status_lines.push(format!("{code}  {path}"));
                stat.push(path);
            }
            Item::IndexWorktree(iw) => match iw {
                index_worktree::Item::Modification {
                    rela_path,
                    status: es,
                    ..
                } => {
                    let code = match es {
                        EntryStatus::Change(WtChange::Removed) => "D",
                        EntryStatus::Change(WtChange::Type { .. }) => "T",
                        EntryStatus::Change(WtChange::Modification { .. }) => "M",
                        EntryStatus::Change(WtChange::SubmoduleModification(_)) => "M",
                        EntryStatus::Conflict { .. } => "U",
                        EntryStatus::NeedsUpdate(_) => continue, // racily-clean, not a change
                        EntryStatus::IntentToAdd => "A",
                    };
                    let path = rela_path.to_string();
                    status_lines.push(format!(" {code} {path}"));
                    if !matches!(es, EntryStatus::NeedsUpdate(_)) {
                        stat.push(path);
                    }
                }
                index_worktree::Item::DirectoryContents { entry, .. } => {
                    use gix::dir::entry::Status as Ds;
                    match entry.status {
                        Ds::Untracked => {
                            let mut p = entry.rela_path.to_string();
                            if entry.disk_kind.map(|k| k.is_dir()).unwrap_or(false)
                                && !p.ends_with('/')
                            {
                                p.push('/');
                            }
                            status_lines.push(format!("?? {p}"));
                        }
                        Ds::Ignored(_) => {} // ignored files aren't in porcelain output
                        _ => {}
                    }
                }
                index_worktree::Item::Rewrite {
                    source,
                    dirwalk_entry,
                    copy,
                    ..
                } => {
                    use index_worktree::RewriteSource;
                    let c = if copy { "C" } else { "R" };
                    let src = match &source {
                        RewriteSource::RewriteFromIndex {
                            source_rela_path, ..
                        } => source_rela_path.to_string(),
                        RewriteSource::CopyFromDirectoryEntry {
                            source_dirwalk_entry,
                            ..
                        } => source_dirwalk_entry.rela_path.to_string(),
                    };
                    status_lines.push(format!(" {c} {src} -> {}", dirwalk_entry.rela_path));
                    stat.push(dirwalk_entry.rela_path.to_string());
                }
            },
        }
    }

    status_lines.sort();
    stat.sort();
    stat.dedup();
    Ok((status_lines, stat))
}

/// The worktree root of the repository containing `path` — the base every
/// [`FileDiff::path`] is relative to, so a caller can translate them to its own
/// root. [`GitError::NotARepo`] outside a worktree.
pub fn worktree_root(path: impl AsRef<Path>) -> Result<PathBuf, GitError> {
    let path = path.as_ref();
    let repo = discover_repository(path)?;
    repo.workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| GitError::NotARepo(path.to_path_buf()))
}

/// Every uncommitted file of the worktree containing `path`, sorted by path.
///
/// "Uncommitted" is one comparison — HEAD's blob against the bytes on disk —
/// so a staged change and a further unstaged edit of the same file collapse
/// into one entry, which is what the changes panel shows. Returns
/// [`GitError::NotARepo`] outside a worktree.
///
/// This compatibility API is NOT a safe scanner for an untrusted worktree.
/// Before previewing files, gix status can hash tracked content, run configured
/// filters, and read ignore/attribute files without our size/nonblocking limits
/// (a FIFO `.gitignore` or `.gitattributes` can block inside status). Retaining
/// Git's ignore and staged/unstaged semantics retains these upstream risks.
///
/// The subsequent preview reads are bounded. On Linux/Android/macOS, each path
/// component is opened relative to a pinned directory with NOFOLLOW/NONBLOCK;
/// other platforms retain a compatibility reader (see `legacy_worktree`).
/// Rejected/unreadable/oversized content produces a truncated row, not a
/// fabricated deletion. This does not bound status, Git object decoding, total
/// scan work, diff CPU time, or filesystem latency.
pub fn worktree_changes(path: impl AsRef<Path>) -> Result<Vec<FileDiff>, GitError> {
    let path = path.as_ref();
    let repo = discover_repository(path)?;
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| GitError::NotARepo(path.to_path_buf()))?;

    let mut paths = changed_paths(&repo)?;
    // Keep the legacy display ordering, but never use the lossy display name
    // for filesystem access or deduplication (two Unix paths may display alike).
    paths.sort_by_cached_key(|path| (path.to_string(), path.clone()));
    paths.dedup();

    // The cap is on the whole reply: keep the first N by path and mark the
    // last one, so the panel can say "and more" without a second count.
    let capped = paths.len() > MAX_CHANGED_FILES;
    paths.truncate(MAX_CHANGED_FILES);

    // Unborn HEAD has no tree: every worktree file reads as an addition.
    let head = head_tree(&repo);
    let reader = legacy_worktree::Root::open(&workdir).map_err(git_io)?;
    let mut files: Vec<FileDiff> = Vec::with_capacity(paths.len());
    for rel in &paths {
        let display = rel.to_string();
        let (old, new) = match gix::path::try_from_byte_slice(rel.as_ref()) {
            Ok(path) if safe_relative(path) => (
                head.as_ref()
                    .map_or(DiskContent::Missing, |tree| blob_at(&repo, tree, path)),
                reader.read(path),
            ),
            _ => (DiskContent::Unavailable, DiskContent::Unavailable),
        };
        let Some(file) = file_diff(&display, old, new) else {
            continue; // both sides identical — nothing to show
        };
        files.push(file);
    }
    if capped {
        if let Some(last) = files.last_mut() {
            last.truncated = true;
        }
    }
    Ok(files)
}

/// Every path whose HEAD version differs from the index or the worktree, plus
/// every untracked file. Untracked entries are taken as `Files`, not
/// `Collapsed`: the panel shows one row per file, and a new directory would
/// otherwise arrive as a single `dir/` row with nothing to diff.
fn changed_paths(repo: &gix::Repository) -> Result<Vec<gix::bstr::BString>, GitError> {
    use gix::status::index_worktree;
    use gix::status::plumbing::index_as_worktree::EntryStatus;
    use gix::status::Item;

    let mut paths: Vec<gix::bstr::BString> = Vec::new();

    let platform = repo
        .status(gix::progress::Discard)
        .map_err(|e| GitError::Op(e.to_string()))?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled);

    for item in platform.into_iter(Vec::<gix::bstr::BString>::new())? {
        let item = match item {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(%e, "status iter error, skipping entry");
                continue;
            }
        };

        match item {
            // Staged: HEAD tree against the index.
            Item::TreeIndex(change) => {
                use gix::diff::index::Change;
                match change {
                    Change::Addition { location, .. }
                    | Change::Deletion { location, .. }
                    | Change::Modification { location, .. } => paths.push(location.into_owned()),
                    // Both ends are paths of their own: the source is gone, the
                    // destination is new — HEAD only knows the source.
                    Change::Rewrite {
                        source_location,
                        location,
                        ..
                    } => {
                        paths.push(source_location.into_owned());
                        paths.push(location.into_owned());
                    }
                }
            }
            Item::IndexWorktree(iw) => match iw {
                index_worktree::Item::Modification {
                    rela_path, status, ..
                } => {
                    if !matches!(status, EntryStatus::NeedsUpdate(_)) {
                        paths.push(rela_path);
                    }
                }
                index_worktree::Item::DirectoryContents { entry, .. } => {
                    // Ignored entries stay out — the walk descends into
                    // untracked dirs but honours .gitignore first.
                    if entry.status == gix::dir::entry::Status::Untracked
                        && !entry.disk_kind.map(|k| k.is_dir()).unwrap_or(false)
                    {
                        paths.push(entry.rela_path);
                    }
                }
                index_worktree::Item::Rewrite {
                    source,
                    dirwalk_entry,
                    ..
                } => {
                    use index_worktree::RewriteSource;
                    match &source {
                        RewriteSource::RewriteFromIndex {
                            source_rela_path, ..
                        } => paths.push(source_rela_path.clone()),
                        RewriteSource::CopyFromDirectoryEntry {
                            source_dirwalk_entry,
                            ..
                        } => paths.push(source_dirwalk_entry.rela_path.clone()),
                    }
                    paths.push(dirwalk_entry.rela_path);
                }
            },
        }
    }
    Ok(paths)
}

/// Read only worktree content below `workspace`, returning repository-relative
/// paths. Unlike `status()`, this never lets gix hash worktree files or read
/// worktree attributes/ignore files: those reads would bypass our safe opener.
///
/// Linux/Android/macOS use descriptor-relative, no-follow, nonblocking opens
/// and descriptor-based directory iteration (no `/proc` dependency). Other
/// platforms (including Windows) return an explicit error before inspecting
/// `workspace` or doing any filesystem/repository I/O: they remain unsupported
/// until an equivalent audited handle-relative backend exists. There is no
/// fallback to the legacy reader. This restriction applies only to this API;
/// [`worktree_changes`] and the other Git entry points retain their behavior.
/// Repository metadata/object storage is trusted; this is not a sandbox for a
/// malicious .git directory, mount changes, or hard links. Renames may produce
/// a mixed-time snapshot, but cannot redirect an opened directory via a symlink.
///
/// Ignore rules are read safely inside this workspace only. Ancestor/global
/// excludes and Git clean filters are deliberately not loaded. Unsafe or
/// oversized entries are conservatively listed with no patch/counts; this may
/// include unchanged entries whose contents could not be safely compared.
pub fn worktree_changes_scoped(workspace: impl AsRef<Path>) -> Result<Vec<FileDiff>, GitError> {
    // Reject unsupported platforms before even interpreting the caller's path:
    // discover/canonicalize could otherwise follow a reparse point or contact
    // a device, pipe, or remote share before the safe opener gets a say.
    safe_worktree::ensure_supported().map_err(git_io)?;
    let workspace = std::path::absolute(workspace.as_ref()).map_err(git_io)?;
    let repo = discover_repository(&workspace)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::NotARepo(workspace.clone()))?;
    let workdir = workdir.canonicalize().map_err(git_io)?;
    let root = safe_worktree::Root::open(&workdir).map_err(git_io)?;
    let prefix = workspace
        .strip_prefix(&workdir)
        .map_err(|_| GitError::Op("workspace is outside the worktree".into()))?;
    if !prefix.as_os_str().is_empty() && !safe_relative(prefix) {
        return Err(GitError::Op("invalid workspace scope".into()));
    }
    let root = root.directory(prefix).map_err(git_io)?;
    let mut paths = BTreeSet::new();
    let mut old = BTreeMap::new();
    // Traverse only the selected HEAD subtree, never sibling blobs.
    let mut head = repo.head().map_err(|e| GitError::Op(e.to_string()))?;
    if !head.is_unborn() {
        let tree = head
            .peel_to_commit()
            .map_err(|e| GitError::Op(e.to_string()))?
            .tree()
            .map_err(|e| GitError::Op(e.to_string()))?;
        let subtree = if prefix.as_os_str().is_empty() {
            Some(tree)
        } else {
            match tree
                .lookup_entry_by_path(prefix)
                .map_err(|e| GitError::Op(e.to_string()))?
            {
                Some(entry) if entry.mode().is_tree() => Some(
                    entry
                        .object()
                        .map_err(|e| GitError::Op(e.to_string()))?
                        .try_into_tree()
                        .map_err(|e| GitError::Op(e.to_string()))?,
                ),
                _ => None,
            }
        };
        if let Some(tree) = subtree {
            collect_head(&tree, Path::new(""), &mut old)?;
        }
    }
    paths.extend(old.keys().cloned());
    let index = repo
        .index_or_empty()
        .map_err(|e| GitError::Op(e.to_string()))?;
    for entry in index.entries() {
        let Some(path) = git_path(entry.path(&index).as_ref()) else {
            continue;
        };
        if let Ok(relative) = path.strip_prefix(prefix) {
            if safe_relative(relative) {
                paths.insert(relative.to_path_buf());
            }
        }
        check_scan_limit(paths.len())?;
    }
    let mut remaining = MAX_SCAN_BYTES;
    walk_workspace(
        &root,
        Path::new(""),
        &mut Vec::new(),
        &mut paths,
        &mut remaining,
        &mut 0,
    )?;
    let mut files: Vec<FileDiff> = Vec::new();
    for path in paths {
        if files.len() == MAX_CHANGED_FILES {
            if let Some(last) = files.last_mut() {
                last.truncated = true;
                last.diff.clear();
                last.adds = 0;
                last.dels = 0;
            }
            break;
        }
        let prior = old.get(&path);
        require_read_budget(remaining)?;
        let new = root.read(&path);
        charge_read(&new, &mut remaining);
        let old_content = match prior {
            None => DiskContent::Missing,
            Some((_, false)) => DiskContent::Unavailable,
            Some((id, true)) => {
                require_read_budget(remaining)?;
                let content = scoped_blob_at(&repo, *id);
                charge_read(&content, &mut remaining);
                content
            }
        };
        // All components have already passed strict UTF-8 validation. On Unix
        // a backslash is a literal filename character, never a separator.
        let rel = prefix.join(&path).to_string_lossy().into_owned();
        if let Some(file) = scoped_file_diff(&rel, old_content, new) {
            files.push(file);
        }
    }
    Ok(files)
}

/// Branch lookup without the worktree-wide status scan used by git_snapshot().
pub fn worktree_branch(path: impl AsRef<Path>) -> Result<Option<String>, GitError> {
    let path = path.as_ref();
    let repo = discover_repository(path)?;
    Ok(current_branch(&repo))
}

fn git_io(error: std::io::Error) -> GitError {
    GitError::Op(error.to_string())
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn git_path(bytes: &[u8]) -> Option<PathBuf> {
    // Never lossy-convert an untrusted Git path into a different disk path.
    let path = PathBuf::from(std::str::from_utf8(bytes).ok()?);
    safe_relative(&path).then_some(path)
}

fn check_scan_limit(count: usize) -> Result<(), GitError> {
    if count > MAX_SCAN_ENTRIES {
        Err(GitError::Op("workspace scan entry limit exceeded".into()))
    } else {
        Ok(())
    }
}

fn require_read_budget(remaining: usize) -> Result<(), GitError> {
    if remaining < MAX_SAFE_DIFF_BYTES as usize + 1 {
        Err(GitError::Op(
            "workspace scan byte limit exceeded; changes are incomplete".into(),
        ))
    } else {
        Ok(())
    }
}

fn collect_head(
    tree: &gix::Tree<'_>,
    prefix: &Path,
    old: &mut BTreeMap<PathBuf, (gix::ObjectId, bool)>,
) -> Result<(), GitError> {
    if prefix.components().count() > 64 {
        return Err(GitError::Op("workspace tree is too deep".into()));
    }
    for entry in tree.iter() {
        let entry = entry.map_err(|e| GitError::Op(e.to_string()))?;
        let Some(name) = git_path(entry.filename().as_ref()) else {
            continue;
        };
        let path = prefix.join(name);
        if entry.mode().is_tree() {
            let subtree = entry
                .object()
                .map_err(|e| GitError::Op(e.to_string()))?
                .try_into_tree()
                .map_err(|e| GitError::Op(e.to_string()))?;
            collect_head(&subtree, &path, old)?;
        } else {
            old.insert(path, (entry.object_id(), entry.mode().is_blob()));
            check_scan_limit(old.len())?;
        }
    }
    Ok(())
}

/// HEAD's tree, or `None` when HEAD is unborn.
fn head_tree(repo: &gix::Repository) -> Option<gix::Tree<'_>> {
    repo.head_commit().ok()?.tree().ok()
}

/// Legacy HEAD side: reject known-large blobs before requesting their bytes.
/// Object lookup (notably packed deltas) is still gix's responsibility and is
/// not a sandboxed/bounded decoder.
fn blob_at(repo: &gix::Repository, tree: &gix::Tree<'_>, rel: &Path) -> DiskContent {
    let entry = match tree.lookup_entry_by_path(rel) {
        Ok(Some(entry)) => entry,
        Ok(None) => return DiskContent::Missing,
        Err(_) => return DiskContent::Unavailable,
    };
    if !entry.mode().is_blob_or_symlink() {
        return DiskContent::Missing;
    }
    let Ok(header) = repo.find_header(entry.object_id()) else {
        return DiskContent::Unavailable;
    };
    if header.kind() != gix::object::Kind::Blob || header.size() > MAX_DIFF_BYTES {
        return DiskContent::Unavailable;
    }
    match entry
        .object()
        .ok()
        .and_then(|object| object.try_into_blob().ok())
    {
        Some(blob) if blob.data.len() as u64 <= MAX_DIFF_BYTES => {
            DiskContent::Bytes(blob.data.clone())
        }
        _ => DiskContent::Unavailable,
    }
}

/// Check the object size before asking gix to inflate/copy the blob.
fn scoped_blob_at(repo: &gix::Repository, id: gix::ObjectId) -> DiskContent {
    let Ok(header) = repo.find_header(id) else {
        return DiskContent::Unavailable;
    };
    if header.size() > MAX_SAFE_DIFF_BYTES {
        return DiskContent::Unavailable;
    }
    let Ok(object) = repo.find_object(id) else {
        return DiskContent::Unavailable;
    };
    let Ok(blob) = object.try_into_blob() else {
        return DiskContent::Unavailable;
    };
    if blob.data.len() as u64 > MAX_SAFE_DIFF_BYTES {
        return DiskContent::Unavailable;
    }
    DiskContent::Bytes(blob.data.clone())
}

/// Missing is distinct from denied/unsafe/oversized: errors must never be
/// rendered as deletions, or generate a patch exposing the previous contents.
enum DiskContent {
    Missing,
    Bytes(Vec<u8>),
    Unavailable,
}

impl DiskContent {
    fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Bytes(bytes) => bytes,
            _ => Vec::new(),
        }
    }
}

fn charge_read(content: &DiskContent, remaining: &mut usize) {
    let charge = match content {
        DiskContent::Bytes(bytes) => bytes.len(),
        DiskContent::Missing => 0,
        DiskContent::Unavailable => MAX_SAFE_DIFF_BYTES as usize + 1,
    };
    *remaining = remaining.saturating_sub(charge);
}

/// Walk metadata below the pinned root only. Ignore files are subject to the
/// same safe reader and limits as previews. Refuse an unreadable ignore file
/// rather than accidentally exposing the files it was supposed to hide.
fn walk_workspace(
    root: &safe_worktree::Root,
    relative: &Path,
    ignores: &mut Vec<ignore::gitignore::Gitignore>,
    paths: &mut BTreeSet<PathBuf>,
    remaining: &mut usize,
    visited: &mut usize,
) -> Result<(), GitError> {
    if relative.components().count() > 64 {
        return Err(GitError::Op("workspace directory is too deep".into()));
    }
    let directory = root.directory(relative).map_err(git_io)?;
    let children = directory.children().map_err(git_io)?;
    // Do not descend into nested repositories (including submodules).
    if !relative.as_os_str().is_empty()
        && children.iter().any(|(name, _)| name == Path::new(".git"))
    {
        return Ok(());
    }
    if *remaining < MAX_SAFE_DIFF_BYTES as usize + 1 {
        return Err(GitError::Op("workspace scan byte limit exceeded".into()));
    }
    let ignore = directory.read(Path::new(".gitignore"));
    charge_read(&ignore, remaining);
    let mut builder = ignore::gitignore::GitignoreBuilder::new(relative);
    match ignore {
        DiskContent::Bytes(bytes) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| GitError::Op("non-UTF-8 workspace .gitignore".into()))?;
            for line in text.lines() {
                builder
                    .add_line(Some(relative.join(".gitignore")), line)
                    .map_err(|e| GitError::Op(e.to_string()))?;
            }
        }
        DiskContent::Missing => {}
        DiskContent::Unavailable => {
            return Err(GitError::Op(
                "unsafe, unreadable or oversized workspace .gitignore".into(),
            ));
        }
    }
    ignores.push(builder.build().map_err(|e| GitError::Op(e.to_string()))?);
    for (name, is_dir) in children {
        *visited += 1;
        check_scan_limit(*visited)?;
        if name == Path::new(".git") {
            continue;
        }
        let path = relative.join(name);
        let ignored = ignores
            .iter()
            .rev()
            .map(|matcher| matcher.matched(&path, is_dir))
            .find(|result| !result.is_none())
            .is_some_and(|result| result.is_ignore());
        if ignored {
            continue;
        }
        if is_dir {
            walk_workspace(root, &path, ignores, paths, remaining, visited)?;
        } else {
            paths.insert(path);
            check_scan_limit(paths.len())?;
        }
    }
    ignores.pop();
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
mod safe_worktree {
    use super::{safe_relative, DiskContent, MAX_SAFE_DIFF_BYTES, MAX_SCAN_ENTRIES};
    use rustix::fs::{openat, Dir, FileType, Mode, OFlags};
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{self, Read};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf};

    pub(super) struct Root(File);

    pub(super) fn ensure_supported() -> io::Result<()> {
        Ok(())
    }

    fn open_child(parent: &File, name: &Path, directory: bool) -> io::Result<File> {
        if !safe_relative(name) || name.components().count() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid child path",
            ));
        }
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        if directory {
            flags |= OFlags::DIRECTORY;
        }
        // A single validated component makes NOFOLLOW apply to every hop.
        // rustix supplies the target's constants and owns the returned fd.
        openat(parent, name, flags, Mode::empty())
            .map(File::from)
            .map_err(io::Error::from)
    }

    impl Root {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "root must be absolute",
                ));
            }
            let mut directory = File::open("/")?;
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => {
                        directory = open_child(&directory, Path::new(name), true)?;
                    }
                    _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid root")),
                }
            }
            Ok(Self(directory))
        }

        pub(super) fn directory(&self, relative: &Path) -> io::Result<Self> {
            if !relative.as_os_str().is_empty() && !safe_relative(relative) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid directory",
                ));
            }
            let mut directory = self.0.try_clone()?;
            for component in relative.components() {
                let Component::Normal(name) = component else {
                    unreachable!()
                };
                directory = open_child(&directory, Path::new(name), true)?;
            }
            Ok(Self(directory))
        }

        pub(super) fn children(&self) -> io::Result<Vec<(PathBuf, bool)>> {
            // rustix opens "." relative to this fd, then uses getdents on
            // Linux or fdopendir/readdir on macOS. No path is reconstructed,
            // and each call gets an independent directory-stream position.
            let directory = Dir::read_from(&self.0).map_err(io::Error::from)?;
            let mut children = Vec::new();
            let mut seen = 0;
            for entry in directory {
                let entry = entry.map_err(io::Error::from)?;
                let bytes = entry.file_name().to_bytes();
                // Unlike std::fs::read_dir, raw directory streams include dots.
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                if seen == MAX_SCAN_ENTRIES {
                    return Err(io::Error::other("workspace scan entry limit exceeded"));
                }
                seen += 1;
                let name = PathBuf::from(OsStr::from_bytes(bytes));
                // Non-UTF-8 names cannot be faithfully represented by FileDiff.
                if name.to_str().is_some() && safe_relative(&name) {
                    let is_dir = match entry.file_type() {
                        FileType::Directory => true,
                        // Some filesystems return DT_UNKNOWN. Probe by safely
                        // opening a directory, never by path metadata followed
                        // by an unchecked open. Links/special files stay leaves.
                        FileType::Unknown => open_child(&self.0, &name, true).is_ok(),
                        _ => false,
                    };
                    children.push((name, is_dir));
                }
            }
            children.sort();
            Ok(children)
        }

        pub(super) fn read(&self, relative: &Path) -> DiskContent {
            if !safe_relative(relative) {
                return DiskContent::Unavailable;
            }
            let Some(name) = relative.file_name() else {
                return DiskContent::Unavailable;
            };
            let parent = match self.directory(relative.parent().unwrap_or(Path::new(""))) {
                Ok(parent) => parent,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return DiskContent::Missing
                }
                Err(_) => return DiskContent::Unavailable,
            };
            // Open first with no-follow + nonblock, then fstat the actual fd:
            // no metadata-then-open race, and a replacement FIFO cannot block.
            let file = match open_child(&parent.0, Path::new(name), false) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return DiskContent::Missing
                }
                Err(_) => return DiskContent::Unavailable,
            };
            let metadata = match file.metadata() {
                Ok(metadata) => metadata,
                Err(_) => return DiskContent::Unavailable,
            };
            if !metadata.is_file() || metadata.len() > MAX_SAFE_DIFF_BYTES {
                return DiskContent::Unavailable;
            }
            let mut bytes = Vec::new();
            // The sentinel also detects files growing after fstat; allocation
            // and actual reads stay bounded independent of reported st_size.
            match file.take(MAX_SAFE_DIFF_BYTES + 1).read_to_end(&mut bytes) {
                Ok(_) if bytes.len() as u64 <= MAX_SAFE_DIFF_BYTES => DiskContent::Bytes(bytes),
                _ => DiskContent::Unavailable,
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
mod safe_worktree {
    use super::DiskContent;
    use std::{
        io,
        path::{Path, PathBuf},
    };

    // Windows security decision (#16): deliberately no filesystem backend.
    //
    // Static audit of local cap-primitives 4.0.3 (matches cached .crate):
    // archive SHA-256:
    // 8b5f74729fd2f44701d1a8eb47e906cdb3ccd9ec0f02baad85a744b791940b18
    // Paths below are relative to its src/:
    // - windows/fs/create_file_at_w.rs uses NtCreateFile + RootDirectory, but
    //   its object attributes do not include OBJ_DONT_REPARSE. OPEN_REPARSE_POINT
    //   is not a blanket refusal of every reparse tag/namespace redirection.
    // - windows/fs/open_unchecked.rs checks metadata on the opened handle
    //   (not a metadata-then-open race), but its nofollow rejection only checks
    //   is_symlink(), not the full FILE_ATTRIBUTE_REPARSE_POINT bit.
    // - windows/fs/open_impl.rs blocks a list of DOS device stems on the final
    //   filename. This is not an all-component namespace/ADS validator: e.g.
    //   "file:stream" has no explicit colon/stream rejection on the open path.
    // - windows/fs/read_dir_inner.rs requires FollowSymlinks::Yes and rebuilds
    //   a pathname via get_path::concatenate before std::fs::read_dir. This is
    //   not handle-bound enumeration. oflags.rs does remove FILE_SHARE_DELETE
    //   for maybe_dir to resist renames; this is not evidence of a proven
    //   exploitable escape, nor proof of our stronger enumeration contract.
    // - create_file_at_w.rs normally selects FILE_SYNCHRONOUS_IO_NONALERT.
    //   Windows oflags.rs does not implement the nonblock option. Neither
    //   FILE_NON_DIRECTORY_FILE nor post-open metadata proves that opening
    //   an unknown object cannot already block or contact a device/pipe.
    //
    // A future backend needs an audited root bootstrap, strict per-component
    // names (including ADS/device rejection), atomic all-reparse refusal,
    // pinned-handle enumeration, and safe object admission BEFORE any open
    // that could block or invoke a device/pipe.
    // RootDirectory + OBJ_DONT_REPARSE alone does not prove device/pipe safety;
    // metadata-then-open or final-path comparisons are not substitutes.
    // Re-enabling requires native Windows tests for junction/symlink and other
    // reparse tags, namespace/ADS aliases, pipe/device hangs, concurrent swaps,
    // and root/ancestor renames. Linux source review/cross-compilation cannot
    // establish those kernel/filesystem properties. Until then, even benign
    // or missing Windows scopes fail before path conversion/discovery/I/O.
    pub(super) struct Root;
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "safe worktree previews require Linux/Android/macOS; no safe opener on this platform",
        )
    }

    pub(super) fn ensure_supported() -> io::Result<()> {
        Err(unsupported())
    }

    impl Root {
        pub(super) fn open(_: &Path) -> io::Result<Self> {
            Err(unsupported())
        }
        pub(super) fn directory(&self, _: &Path) -> io::Result<Self> {
            Err(unsupported())
        }
        pub(super) fn children(&self) -> io::Result<Vec<(PathBuf, bool)>> {
            Err(unsupported())
        }
        pub(super) fn read(&self, _: &Path) -> DiskContent {
            DiskContent::Unavailable
        }
    }
}

/// Only the legacy preview stage: status/config/object reads happen elsewhere.
/// This is deliberately independent of the scoped API and its smaller limits.
mod legacy_worktree {
    use super::{safe_relative, DiskContent, MAX_DIFF_BYTES};
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::Path;

    fn content(file: io::Result<File>) -> DiskContent {
        let file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return DiskContent::Missing,
            Err(_) => return DiskContent::Unavailable,
        };
        // Check the opened object, NOT metadata(path) followed by open(path).
        // With Unix NONBLOCK this also rejects FIFOs without waiting for a writer.
        let metadata = match file.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => return DiskContent::Unavailable,
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            // Reject every final reparse point, not just symbolic links.
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return DiskContent::Unavailable;
            }
        }
        if metadata.len() > MAX_DIFF_BYTES {
            return DiskContent::Unavailable;
        }
        // The descriptor may grow after fstat. Limit the actual read as well;
        // the extra byte distinguishes an exact-limit file from an oversized one.
        let mut bytes = Vec::new();
        match file.take(MAX_DIFF_BYTES + 1).read_to_end(&mut bytes) {
            Ok(_) if bytes.len() as u64 <= MAX_DIFF_BYTES => DiskContent::Bytes(bytes),
            _ => DiskContent::Unavailable,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    mod platform {
        use super::*;
        use rustix::fd::OwnedFd;
        use rustix::fs::{openat, Mode, OFlags, CWD};
        use std::path::Component;

        const READ: OFlags = OFlags::RDONLY
            .union(OFlags::CLOEXEC)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::NONBLOCK);
        const DIRECTORY: OFlags = READ.union(OFlags::DIRECTORY);

        pub(in super::super) struct Root(OwnedFd);

        impl Root {
            pub(in super::super) fn open(path: &Path) -> io::Result<Self> {
                // Root aliases (e.g. macOS /var -> /private/var) were accepted by
                // the legacy API. Resolve only this caller-selected root; after
                // that, pin it without following substituted path components.
                let absolute = path.canonicalize()?;
                let mut fd = openat(CWD, "/", DIRECTORY, Mode::empty())?;
                for component in absolute.components() {
                    match component {
                        Component::RootDir => {}
                        Component::Normal(name) => {
                            fd = openat(&fd, name, DIRECTORY, Mode::empty())?;
                        }
                        _ => return Err(io::Error::other("invalid worktree root")),
                    }
                }
                Ok(Self(fd))
            }

            fn file(&self, path: &Path) -> io::Result<File> {
                let mut components = path.components().peekable();
                let mut directory = openat(&self.0, ".", DIRECTORY, Mode::empty())?;
                while let Some(Component::Normal(name)) = components.next() {
                    if components.peek().is_none() {
                        return Ok(openat(&directory, name, READ, Mode::empty())?.into());
                    }
                    directory = openat(&directory, name, DIRECTORY, Mode::empty())?;
                }
                Err(io::Error::other("invalid relative worktree path"))
            }

            pub(in super::super) fn read(&self, path: &Path) -> DiskContent {
                if !safe_relative(path) {
                    return DiskContent::Unavailable;
                }
                content(self.file(path))
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    mod platform {
        use super::*;
        use std::fs::OpenOptions;
        use std::path::PathBuf;

        pub(in super::super) struct Root(PathBuf);

        impl Root {
            pub(in super::super) fn open(path: &Path) -> io::Result<Self> {
                Ok(Self(path.canonicalize()?))
            }

            pub(in super::super) fn read(&self, path: &Path) -> DiskContent {
                if !safe_relative(path) {
                    return DiskContent::Unavailable;
                }
                let mut options = OpenOptions::new();
                options.read(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
                    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
                }
                // Compatibility, NOT full containment/nonblocking: on Windows
                // final reparse points are not followed, but ancestor reparse
                // points can still redirect. Other fallback targets may follow
                // even the final symlink. Opens/reads can block on these targets.
                // Never replace this with the scoped API's unsupported stub:
                // normal Windows Tauri files must remain readable.
                content(options.open(self.0.join(path)))
            }
        }
    }

    pub(super) use platform::Root;
}

/// Git's binary heuristic: a NUL byte in the head of the file.
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_LEN).any(|b| *b == 0)
}

/// Build one entry, or `None` when old and new bytes are identical (a copied
/// or racily-clean path the status walk still lists).
fn file_diff(rel: &str, old: DiskContent, new: DiskContent) -> Option<FileDiff> {
    let kind = match (&old, &new) {
        (DiskContent::Missing, _) => ChangeKind::Add,
        (_, DiskContent::Missing) => ChangeKind::Delete,
        _ => ChangeKind::Update,
    };
    let mut file = FileDiff {
        path: rel.to_string(),
        kind,
        adds: 0,
        dels: 0,
        diff: String::new(),
        binary: false,
        truncated: false,
    };
    let (old, new) = match (old, new) {
        (DiskContent::Unavailable, _) | (_, DiskContent::Unavailable) => {
            file.truncated = true;
            return Some(file);
        }
        (old, new) => (old.into_bytes(), new.into_bytes()),
    };
    if old == new {
        return None;
    }

    // A NUL byte up front is git's own answer; non-UTF-8 content is ours,
    // since a line diff needs text on both sides to align.
    if looks_binary(&old) || looks_binary(&new) {
        file.binary = true;
        return Some(file);
    }
    let (old_text, new_text) = match (std::str::from_utf8(&old), std::str::from_utf8(&new)) {
        (Ok(o), Ok(n)) => (o, n),
        _ => {
            file.binary = true;
            return Some(file);
        }
    };

    // Too big to diff on an IPC call: the path and the counts alone. Counting
    // the delta would mean doing the very diff we just refused.
    if old.len() as u64 > MAX_DIFF_BYTES || new.len() as u64 > MAX_DIFF_BYTES {
        file.truncated = true;
        return Some(file);
    }

    let text = TextDiff::from_lines(old_text, new_text);
    for op in text.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {}
            similar::DiffTag::Insert => file.adds += op.new_range().len() as u32,
            similar::DiffTag::Delete => file.dels += op.old_range().len() as u32,
            similar::DiffTag::Replace => {
                file.adds += op.new_range().len() as u32;
                file.dels += op.old_range().len() as u32;
            }
        }
    }
    file.diff = text
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{rel}"), &format!("b/{rel}"))
        .to_string();
    Some(file)
}

/// Build one entry, or `None` when old and new bytes are identical (a copied
/// or racily-clean path the status walk still lists).
fn scoped_file_diff(rel: &str, old: DiskContent, new: DiskContent) -> Option<FileDiff> {
    let kind = match (&old, &new) {
        (DiskContent::Missing, _) => ChangeKind::Add,
        (_, DiskContent::Missing) => ChangeKind::Delete,
        _ => ChangeKind::Update,
    };
    let mut file = FileDiff {
        path: rel.to_string(),
        kind,
        adds: 0,
        dels: 0,
        diff: String::new(),
        binary: false,
        truncated: false,
    };
    let (old, new) = match (old, new) {
        (DiskContent::Unavailable, _) | (_, DiskContent::Unavailable) => {
            file.truncated = true;
            return Some(file);
        }
        (DiskContent::Missing, DiskContent::Missing) => return None,
        (DiskContent::Bytes(old), DiskContent::Bytes(new)) if old == new => return None,
        (old, new) => (old.into_bytes(), new.into_bytes()),
    };

    // A NUL byte up front is git's own answer; non-UTF-8 content is ours,
    // since a line diff needs text on both sides to align.
    if looks_binary(&old) || looks_binary(&new) {
        file.binary = true;
        return Some(file);
    }
    let (old_text, new_text) = match (std::str::from_utf8(&old), std::str::from_utf8(&new)) {
        (Ok(o), Ok(n)) => (o, n),
        _ => {
            file.binary = true;
            return Some(file);
        }
    };

    // Too big to diff on an IPC call: the path and the counts alone. Counting
    // the delta would mean doing the very diff we just refused.
    if old.len() as u64 > MAX_SAFE_DIFF_BYTES || new.len() as u64 > MAX_SAFE_DIFF_BYTES {
        file.truncated = true;
        return Some(file);
    }

    let text = TextDiff::configure()
        .timeout(std::time::Duration::from_millis(100))
        .diff_lines(old_text, new_text);
    for op in text.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {}
            similar::DiffTag::Insert => file.adds += op.new_range().len() as u32,
            similar::DiffTag::Delete => file.dels += op.old_range().len() as u32,
            similar::DiffTag::Replace => {
                file.adds += op.new_range().len() as u32;
                file.dels += op.old_range().len() as u32;
            }
        }
    }
    file.diff = text
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{rel}"), &format!("b/{rel}"))
        .to_string();
    Some(file)
}

/// Trim sections until the rendered block fits [`GIT_SNAPSHOT_BUDGET`].
fn enforce_budget(snap: &mut GitSnapshot) {
    while snap.to_prompt_block().len() > GIT_SNAPSHOT_BUDGET {
        if snap.diff_stat.len() > 1 {
            snap.diff_stat.pop();
        } else if snap.status.len() > 1 {
            snap.status.pop();
        } else {
            break;
        }
        snap.truncated = true;
    }
    if snap.truncated {
        // Marker fits too — to_prompt_block() includes it once set.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn init_repo() -> tempfile::TempDir {
        // macOS temporary paths commonly start with the /var -> /private/var
        // system alias. Use a canonical test root; do not weaken the reader's
        // refusal of symlink components to accommodate the fixture.
        #[cfg(target_os = "macos")]
        let dir = tempfile::Builder::new()
            .tempdir_in(std::env::temp_dir().canonicalize().unwrap())
            .unwrap();
        #[cfg(not(target_os = "macos"))]
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", dir.path())
                .output()
                .unwrap();
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        match git_snapshot(dir.path()) {
            Err(GitError::NotARepo(_)) => {}
            other => panic!("expected NotARepo, got {other:?}"),
        }
    }

    #[test]
    fn discovery_failures_are_not_reported_as_plain_folders() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(matches!(worktree_root(&missing), Err(GitError::Op(_))));
        assert!(matches!(worktree_changes(&missing), Err(GitError::Op(_))));

        let repo = init_repo();
        fs::write(repo.path().join(".git/config"), b"[broken\n").unwrap();
        assert!(matches!(worktree_root(repo.path()), Err(GitError::Op(_))));
        assert!(matches!(
            worktree_changes(repo.path()),
            Err(GitError::Op(_))
        ));
    }

    #[test]
    fn clean_repo() {
        let dir = init_repo();
        let snap = git_snapshot(dir.path()).unwrap();
        assert!(snap.status.is_empty(), "status: {:?}", snap.status);
        assert!(snap.branch.is_some());
    }

    #[test]
    fn detects_modified_and_untracked() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(dir.path().join("new.rs"), "fn main() {}\n").unwrap();
        let snap = git_snapshot(dir.path()).unwrap();
        assert!(
            snap.status
                .iter()
                .any(|l| l.starts_with(" M") && l.contains("a.txt")),
            "status: {:?}",
            snap.status
        );
        assert!(
            snap.status
                .iter()
                .any(|l| l.starts_with("??") && l.contains("new.rs")),
            "status: {:?}",
            snap.status
        );
        assert_eq!(snap.dirty_count(), 2);
        assert!(snap.to_prompt_block().contains("a.txt"));
    }

    #[test]
    fn staged_change_shows_in_index_column() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        Command::new("git")
            .args(["add", "a.txt"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir.path())
            .output()
            .unwrap();
        let snap = git_snapshot(dir.path()).unwrap();
        assert!(
            snap.status
                .iter()
                .any(|l| l.starts_with("M ") && l.contains("a.txt")),
            "status: {:?}",
            snap.status
        );
    }

    #[test]
    fn budget_enforced() {
        let dir = init_repo();
        for i in 0..500 {
            fs::write(
                dir.path().join(format!(
                    "very_long_filename_to_bloat_the_status_block_{i:04}.txt"
                )),
                "x\n",
            )
            .unwrap();
        }
        let snap = git_snapshot(dir.path()).unwrap();
        assert!(
            snap.to_prompt_block().len() <= GIT_SNAPSHOT_BUDGET + 64,
            "len = {}",
            snap.to_prompt_block().len()
        );
        assert!(snap.truncated);
    }

    /// Run git in `dir` — the same hermetic env `init_repo` uses.
    fn git_in(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn find<'a>(files: &'a [FileDiff], path: &str) -> &'a FileDiff {
        files.iter().find(|f| f.path == path).unwrap_or_else(|| {
            panic!(
                "no {path} in {:?}",
                files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        })
    }

    #[test]
    fn changes_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        match worktree_changes(dir.path()) {
            Err(GitError::NotARepo(_)) => {}
            other => panic!("expected NotARepo, got {other:?}"),
        }
    }

    #[test]
    fn worktree_root_answers_the_repo_for_a_subdirectory() {
        let dir = init_repo();
        let sub = dir.path().join("crate");
        fs::create_dir(&sub).unwrap();
        let root = worktree_root(&sub).unwrap().canonicalize().unwrap();
        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn appended_line_is_one_add() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        let f = find(&files, "a.txt");
        assert_eq!(f.kind, ChangeKind::Update);
        assert_eq!((f.adds, f.dels), (1, 0), "diff: {}", f.diff);
        assert!(!f.binary && !f.truncated);
        assert!(f.diff.contains("+two"), "diff: {}", f.diff);
        assert!(f.diff.contains("a/a.txt"), "diff: {}", f.diff);
    }

    #[test]
    fn deleted_file_is_a_delete() {
        let dir = init_repo();
        fs::remove_file(dir.path().join("a.txt")).unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        let f = find(&files, "a.txt");
        assert_eq!(f.kind, ChangeKind::Delete);
        assert_eq!((f.adds, f.dels), (0, 1), "diff: {}", f.diff);
        assert!(f.diff.contains("-one"), "diff: {}", f.diff);
    }

    #[test]
    fn untracked_files_in_a_new_dir_are_listed_individually() {
        let dir = init_repo();
        fs::create_dir(dir.path().join("newdir")).unwrap();
        fs::write(dir.path().join("newdir/b.txt"), "b\n").unwrap();
        fs::write(dir.path().join("newdir/c.txt"), "c\nc2\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        let b = find(&files, "newdir/b.txt");
        let c = find(&files, "newdir/c.txt");
        assert_eq!((b.kind, b.adds, b.dels), (ChangeKind::Add, 1, 0));
        assert_eq!((c.kind, c.adds, c.dels), (ChangeKind::Add, 2, 0));
        assert!(
            files.iter().all(|f| !f.path.ends_with('/')),
            "collapsed dir entry: {:?}",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn staged_change_is_still_listed() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        git_in(dir.path(), &["add", "a.txt"]);
        let files = worktree_changes(dir.path()).unwrap();
        let f = find(&files, "a.txt");
        assert_eq!(f.kind, ChangeKind::Update);
        assert_eq!((f.adds, f.dels), (1, 0));
    }

    #[test]
    fn binary_file_is_flagged_without_a_diff() {
        let dir = init_repo();
        fs::write(dir.path().join("blob.bin"), b"a\x00b\n").unwrap();
        git_in(dir.path(), &["add", "blob.bin"]);
        git_in(dir.path(), &["commit", "-qm", "bin"]);
        fs::write(dir.path().join("blob.bin"), b"a\x00c\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        let f = find(&files, "blob.bin");
        assert!(f.binary, "expected binary, got {f:?}");
        assert_eq!((f.adds, f.dels), (0, 0));
        assert!(f.diff.is_empty(), "diff: {}", f.diff);
    }

    #[test]
    fn unborn_head_reads_everything_as_added() {
        let dir = tempfile::tempdir().unwrap();
        git_in(dir.path(), &["init", "-q"]);
        fs::write(dir.path().join("fresh.txt"), "x\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        let f = find(&files, "fresh.txt");
        assert_eq!((f.kind, f.adds, f.dels), (ChangeKind::Add, 1, 0));
    }

    #[test]
    fn legacy_changes_compare_head_to_disk_for_mixed_staging() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "staged\n").unwrap();
        git_in(dir.path(), &["add", "a.txt"]);
        fs::write(dir.path().join("a.txt"), "one\nunstaged\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let file = find(&files, "a.txt");
        assert_eq!(
            (file.kind, file.adds, file.dels),
            (ChangeKind::Update, 1, 0)
        );
        assert!(file.diff.contains("+unstaged"));
        assert!(!file.diff.contains("+staged\n") && !file.diff.contains("-staged\n"));

        // An index-only change that has been undone on disk is not a diff.
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        assert!(worktree_changes(dir.path()).unwrap().is_empty());
        git_in(dir.path(), &["rm", "-f", "a.txt"]);
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        assert!(worktree_changes(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn legacy_changes_keep_git_ignore_and_tracked_file_semantics() {
        let dir = init_repo();
        fs::write(dir.path().join(".gitignore"), "*.log\n!keep.log\na.txt\n").unwrap();
        fs::write(dir.path().join(".git/info/exclude"), "info-only\n").unwrap();
        git_in(dir.path(), &["add", ".gitignore"]);
        git_in(dir.path(), &["commit", "-qm", "ignore rules"]);
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/.gitignore"), "hidden\n").unwrap();
        for name in [
            "ignored.log",
            "keep.log",
            "info-only",
            "nested/hidden",
            "nested/shown",
        ] {
            fs::write(dir.path().join(name), "content\n").unwrap();
        }
        fs::write(dir.path().join("a.txt"), "one\ntracked edit\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        assert!(files.iter().any(|file| file.path == "a.txt"));
        assert!(files.iter().any(|file| file.path == "keep.log"));
        assert!(files.iter().any(|file| file.path == "nested/shown"));
        for name in ["ignored.log", "info-only", "nested/hidden"] {
            assert!(files.iter().all(|file| file.path != name), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn legacy_changes_preserve_non_utf8_paths_for_both_head_and_disk() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let dir = init_repo();
        let raw = OsString::from_vec(b"name-\xff".to_vec());
        let display = raw.to_string_lossy().into_owned();
        fs::write(dir.path().join(&raw), "raw baseline\n").unwrap();
        fs::write(
            dir.path().join(&display),
            "different literal replacement name\n",
        )
        .unwrap();
        git_in(dir.path(), &["add", "."]);
        git_in(dir.path(), &["commit", "-qm", "raw filename"]);
        fs::write(dir.path().join(raw), "raw edited\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let file = find(&files, &display);
        assert_eq!(file.kind, ChangeKind::Update);
        assert!(file.diff.contains("-raw baseline"));
        assert!(file.diff.contains("+raw edited"));
        assert!(!file.diff.contains("replacement name"));
    }

    #[cfg(unix)]
    #[test]
    fn legacy_changes_accept_a_symlink_alias_for_the_worktree_root() {
        use std::os::unix::fs::symlink;

        let dir = init_repo();
        let aliases = tempfile::tempdir().unwrap();
        let alias = aliases.path().join("repo-alias");
        symlink(dir.path(), &alias).unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), "one\nthrough alias\n").unwrap();
        let files = worktree_changes(alias.join("sub")).unwrap();
        assert!(find(&files, "a.txt").diff.contains("+through alias"));
    }

    #[test]
    fn legacy_rejected_content_is_truncated_not_a_fabricated_deletion() {
        let file = file_diff(
            "tracked",
            DiskContent::Bytes(b"before\n".to_vec()),
            DiskContent::Unavailable,
        )
        .unwrap();
        assert_eq!(file.kind, ChangeKind::Update);
        assert!(file.truncated && file.diff.is_empty());
        assert_eq!((file.adds, file.dels), (0, 0));
        let file = file_diff("new", DiskContent::Missing, DiskContent::Unavailable).unwrap();
        assert_eq!(file.kind, ChangeKind::Add);
        assert!(file.truncated);
    }

    #[test]
    fn legacy_reader_bounds_regular_files_and_distinguishes_missing() {
        let dir = init_repo();
        let exact = dir.path().join("exact");
        fs::write(&exact, vec![b'x'; MAX_DIFF_BYTES as usize]).unwrap();
        let reader = legacy_worktree::Root::open(dir.path()).unwrap();
        match reader.read(Path::new("exact")) {
            DiskContent::Bytes(bytes) => assert_eq!(bytes.len() as u64, MAX_DIFF_BYTES),
            _ => panic!("an exact-limit regular file should remain readable"),
        }
        fs::OpenOptions::new()
            .write(true)
            .open(exact)
            .unwrap()
            .set_len(MAX_DIFF_BYTES + 1)
            .unwrap();
        assert!(matches!(
            reader.read(Path::new("exact")),
            DiskContent::Unavailable
        ));
        assert!(matches!(
            reader.read(Path::new("missing/child")),
            DiskContent::Missing
        ));
        assert!(matches!(
            reader.read(Path::new("../a.txt")),
            DiskContent::Unavailable
        ));
        assert!(matches!(reader.read(dir.path()), DiskContent::Unavailable));
        assert!(matches!(
            reader.read(Path::new("")),
            DiskContent::Unavailable
        ));
    }

    #[test]
    fn legacy_changes_do_not_inflate_known_oversized_head_blobs_for_diff() {
        let dir = init_repo();
        fs::write(
            dir.path().join("large"),
            vec![b'x'; MAX_DIFF_BYTES as usize + 1],
        )
        .unwrap();
        git_in(dir.path(), &["add", "large"]);
        git_in(dir.path(), &["commit", "-qm", "large baseline"]);
        fs::write(dir.path().join("large"), "small\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        let file = find(&files, "large");
        assert_eq!(file.kind, ChangeKind::Update);
        assert!(file.truncated && file.diff.is_empty());
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn legacy_reader_rejects_symlink_components_and_non_regular_files() {
        use std::os::unix::fs::symlink;

        let dir = init_repo();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "outside secret\n").unwrap();
        symlink(outside.path().join("secret"), dir.path().join("link")).unwrap();
        symlink(outside.path(), dir.path().join("ancestor")).unwrap();
        fs::create_dir(dir.path().join("directory")).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(dir.path().join("fifo"))
            .status()
            .unwrap()
            .success());
        let reader = legacy_worktree::Root::open(dir.path()).unwrap();
        for path in ["link", "ancestor/secret", "directory", "fifo"] {
            assert!(
                matches!(reader.read(Path::new(path)), DiskContent::Unavailable),
                "{path}"
            );
        }
        // Public API marks refused symlinks as unavailable, never leaks targets.
        // No FIFO ignore/attribute files here: those can hang in gix itself.
        let files = worktree_changes(dir.path()).unwrap();
        let file = find(&files, "link");
        assert_eq!(file.kind, ChangeKind::Add);
        assert!(file.truncated && file.diff.is_empty());
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn legacy_reader_pins_root_and_rejects_a_replaced_ancestor() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        let moved = parent.path().join("moved");
        let outside = parent.path().join("outside");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("file"), "pinned\n").unwrap();
        fs::write(root.join("nested/file"), "inside\n").unwrap();
        fs::write(outside.join("file"), "outside secret\n").unwrap();
        let reader = legacy_worktree::Root::open(&root).unwrap();
        fs::rename(&root, &moved).unwrap();
        symlink(&outside, &root).unwrap();
        match reader.read(Path::new("file")) {
            DiskContent::Bytes(bytes) => assert_eq!(bytes, b"pinned\n"),
            _ => panic!("root descriptor was not retained"),
        }
        fs::remove_dir_all(moved.join("nested")).unwrap();
        symlink(&outside, moved.join("nested")).unwrap();
        assert!(matches!(
            reader.read(Path::new("nested/file")),
            DiskContent::Unavailable
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn scoped_changes_read_only_the_workspace_and_preserve_diff_semantics() {
        let dir = init_repo();
        std::fs::create_dir(dir.path().join("workspace")).unwrap();
        std::fs::write(dir.path().join("workspace/text"), "before\n").unwrap();
        std::fs::write(dir.path().join("workspace/deleted"), "deleted\n").unwrap();
        std::fs::write(dir.path().join("sibling"), "outside before\n").unwrap();
        git_in(dir.path(), &["add", "."]);
        git_in(dir.path(), &["commit", "-qm", "scoped baseline"]);
        std::fs::write(dir.path().join("workspace/text"), "before\nafter\n").unwrap();
        std::fs::remove_file(dir.path().join("workspace/deleted")).unwrap();
        std::fs::write(dir.path().join("workspace/binary"), b"binary\0data").unwrap();
        std::fs::write(dir.path().join("sibling"), "outside secret\n").unwrap();
        let files = worktree_changes_scoped(dir.path().join("workspace")).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(find(&files, "workspace/text").adds, 1);
        assert_eq!(find(&files, "workspace/deleted").kind, ChangeKind::Delete);
        assert!(find(&files, "workspace/binary").binary);
        assert!(files.iter().all(|file| file.path.starts_with("workspace/")));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn scoped_changes_refuse_links_and_special_files_before_reading() {
        use std::os::unix::{fs::symlink, net::UnixListener};

        let dir = init_repo();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "outside secret\n").unwrap();
        std::fs::create_dir(dir.path().join("workspace")).unwrap();
        std::fs::write(dir.path().join("workspace/replaced"), "tracked\n").unwrap();
        std::fs::write(dir.path().join("workspace/pipe"), "tracked\n").unwrap();
        symlink(
            outside.path().join("secret"),
            dir.path().join("workspace/link"),
        )
        .unwrap();
        git_in(dir.path(), &["add", "."]);
        git_in(dir.path(), &["commit", "-qm", "links baseline"]);
        std::fs::remove_file(dir.path().join("workspace/replaced")).unwrap();
        symlink(
            outside.path().join("secret"),
            dir.path().join("workspace/replaced"),
        )
        .unwrap();
        symlink(outside.path(), dir.path().join("workspace/linked-dir")).unwrap();
        symlink(
            outside.path().join("secret"),
            dir.path().join("workspace/untracked-link"),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("workspace/pipe")).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(dir.path().join("workspace/pipe"))
            .status()
            .unwrap()
            .success());
        let _socket = UnixListener::bind(dir.path().join("workspace/socket")).unwrap();
        // An out-of-scope tracked FIFO would hang an unrestricted status hash.
        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(dir.path().join("a.txt"))
            .status()
            .unwrap()
            .success());
        let files = worktree_changes_scoped(dir.path().join("workspace")).unwrap();
        for path in [
            "workspace/link",
            "workspace/replaced",
            "workspace/pipe",
            "workspace/socket",
            "workspace/untracked-link",
            "workspace/linked-dir",
        ] {
            let file = find(&files, path);
            assert!(file.truncated, "{path}");
            assert!(file.diff.is_empty(), "{path}");
            assert_eq!((file.adds, file.dels), (0, 0), "{path}");
        }
        assert!(!files
            .iter()
            .any(|file| file.diff.contains("outside secret")));
        // A deleted tracked symlink must not turn its stored link target into a patch.
        std::fs::remove_file(dir.path().join("workspace/link")).unwrap();
        let deleted = worktree_changes_scoped(dir.path().join("workspace")).unwrap();
        let file = find(&deleted, "workspace/link");
        assert_eq!(file.kind, ChangeKind::Delete);
        assert!(file.diff.is_empty());
        assert!(file.truncated);
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn scoped_changes_bound_both_blob_and_disk_reads() {
        let dir = init_repo();
        let large = vec![b'x'; MAX_SAFE_DIFF_BYTES as usize + 1];
        std::fs::write(dir.path().join("large-old"), &large).unwrap();
        git_in(dir.path(), &["add", "."]);
        git_in(dir.path(), &["commit", "-qm", "large baseline"]);
        std::fs::write(dir.path().join("large-old"), "small\n").unwrap();
        std::fs::write(dir.path().join("large-new"), large).unwrap();
        let files = worktree_changes_scoped(dir.path()).unwrap();
        for path in ["large-old", "large-new"] {
            let file = find(&files, path);
            assert!(file.truncated);
            assert!(file.diff.is_empty());
            assert_eq!((file.adds, file.dels), (0, 0));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn safe_reader_rejects_escape_paths_and_pins_the_open_directory() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("workspace")).unwrap();
        std::fs::write(dir.path().join("workspace/text"), "inside").unwrap();
        std::fs::write(outside.path().join("text"), "outside").unwrap();
        let root = safe_worktree::Root::open(&dir.path().join("workspace").canonicalize().unwrap())
            .unwrap();
        for path in ["../text", "/etc/passwd", "a/../../text", "", ".", "nul\0"] {
            assert!(matches!(
                root.read(Path::new(path)),
                DiskContent::Unavailable
            ));
        }
        for path in ["../text", "/etc", "a/../../text", ".", "nul\0"] {
            assert!(root.directory(Path::new(path)).is_err(), "{path:?}");
        }
        std::fs::rename(dir.path().join("workspace"), dir.path().join("moved")).unwrap();
        symlink(outside.path(), dir.path().join("workspace")).unwrap();
        match root.read(Path::new("text")) {
            DiskContent::Bytes(bytes) => assert_eq!(bytes, b"inside"),
            _ => panic!("the already opened workspace must stay pinned"),
        }
        symlink(outside.path(), dir.path().join("moved/escape")).unwrap();
        assert!(matches!(
            root.read(Path::new("escape/text")),
            DiskContent::Unavailable
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn safe_reader_enumerates_pinned_directories_with_independent_streams() {
        use std::ffi::OsStr;
        use std::os::unix::{ffi::OsStrExt, fs::symlink};

        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        fs::create_dir_all(base.join("workspace/nested")).unwrap();
        fs::create_dir(base.join("outside")).unwrap();
        fs::write(base.join("workspace/nested/text"), "inside").unwrap();
        fs::write(base.join("outside/secret"), "outside secret").unwrap();
        let root = safe_worktree::Root::open(&base.join("workspace")).unwrap();
        let nested = root.directory(Path::new("nested")).unwrap();
        assert_eq!(
            root.children().unwrap(),
            vec![(PathBuf::from("nested"), true)]
        );
        fs::rename(base.join("workspace"), base.join("moved")).unwrap();
        symlink(base.join("outside"), base.join("workspace")).unwrap();
        fs::rename(base.join("moved/nested"), base.join("moved/pinned")).unwrap();
        symlink(base.join("outside"), base.join("moved/nested")).unwrap();
        fs::write(
            base.join("moved/pinned")
                .join(OsStr::from_bytes(b"bad-\xff")),
            "unrepresentable",
        )
        .unwrap();
        // Both the workspace and a nested directory were swapped for links.
        // Enumerating must use the original handles, omit dots/non-UTF-8 names,
        // and restart from the beginning even when a cloned fd was enumerated.
        let cloned = nested.directory(Path::new("")).unwrap();
        for directory in [&nested, &cloned, &nested] {
            assert_eq!(
                directory.children().unwrap(),
                vec![(PathBuf::from("text"), false)]
            );
            match directory.read(Path::new("text")) {
                DiskContent::Bytes(bytes) => assert_eq!(bytes, b"inside"),
                _ => panic!("the nested directory handle must remain pinned"),
            }
        }
        assert_eq!(
            root.children().unwrap(),
            vec![
                (PathBuf::from("nested"), false),
                (PathBuf::from("pinned"), true),
            ]
        );
        assert!(root.directory(Path::new("nested")).is_err());
        assert!(matches!(
            root.read(Path::new("nested/secret")),
            DiskContent::Unavailable
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn scoped_changes_reject_symlink_scopes_and_ignore_files() {
        use std::os::unix::fs::symlink;

        let dir = init_repo();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("workspace")).unwrap();
        fs::write(outside.path().join("ignore"), "secret\n").unwrap();
        fs::write(dir.path().join("workspace/secret"), "must not be exposed").unwrap();
        symlink(
            dir.path().join("workspace"),
            dir.path().join("linked-workspace"),
        )
        .unwrap();
        assert!(worktree_changes_scoped(dir.path().join("linked-workspace")).is_err());
        symlink(
            outside.path().join("ignore"),
            dir.path().join("workspace/.gitignore"),
        )
        .unwrap();
        match worktree_changes_scoped(dir.path().join("workspace")) {
            Err(GitError::Op(message)) => assert!(message.contains(".gitignore"), "{message}"),
            other => panic!("expected an unsafe-ignore error, not file contents: {other:?}"),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_safe_reader_rejects_root_symlinks_at_every_component() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        fs::create_dir_all(base.join("real/nested")).unwrap();
        fs::write(base.join("real/nested/text"), "inside").unwrap();
        symlink(base.join("real"), base.join("alias")).unwrap();
        // In particular, do not silently canonicalize macOS system aliases
        // such as /var or /tmp inside the safe opener.
        assert!(safe_worktree::Root::open(&base.join("alias")).is_err());
        assert!(safe_worktree::Root::open(&base.join("alias/nested")).is_err());
        let root = safe_worktree::Root::open(&base.join("real")).unwrap();
        assert_eq!(
            root.children().unwrap(),
            vec![(PathBuf::from("nested"), true)]
        );
        assert!(matches!(
            root.read(Path::new("nested/text")),
            DiskContent::Bytes(bytes) if bytes == b"inside"
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn scoped_changes_keep_literal_backslashes_and_report_corrupt_head() {
        let dir = init_repo();
        fs::write(dir.path().join(r"literal\name"), "one\n").unwrap();
        let files = worktree_changes_scoped(dir.path()).unwrap();
        assert_eq!(find(&files, r"literal\name").kind, ChangeKind::Add);
        fs::write(
            dir.path().join(".git/HEAD"),
            "0000000000000000000000000000000000000001\n",
        )
        .unwrap();
        assert!(matches!(
            worktree_changes_scoped(dir.path()),
            Err(GitError::Op(_))
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn scoped_changes_fail_explicitly_when_clean_files_exhaust_budget() {
        let dir = init_repo();
        // More than 16 MiB when both old and new are compared; no file changed.
        let bytes = vec![b'x'; MAX_SAFE_DIFF_BYTES as usize / 2];
        for index in 0..140 {
            fs::write(dir.path().join(format!("clean-{index:03}")), &bytes).unwrap();
        }
        git_in(dir.path(), &["add", "."]);
        git_in(dir.path(), &["commit", "-qm", "budget baseline"]);
        match worktree_changes_scoped(dir.path()) {
            Err(GitError::Op(message)) => assert!(message.contains("byte limit"), "{message}"),
            other => panic!("expected an explicit scan budget error, not file rows: {other:?}"),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    #[test]
    fn scoped_changes_explain_the_unsupported_platform_without_affecting_legacy() {
        let dir = init_repo();
        assert!(worktree_changes(dir.path()).unwrap().is_empty());
        match worktree_changes_scoped(dir.path()) {
            Err(GitError::Op(message)) => assert!(message.contains("safe opener"), "{message}"),
            other => panic!("expected an explicit unsupported-platform error: {other:?}"),
        }
        // Only the scoped entry point is disabled, not ordinary Windows Git
        // functionality. Check both clean metadata and legacy content reads
        // after refusal so a future global/platform gate cannot hide here.
        assert!(worktree_root(dir.path()).is_ok());
        assert!(worktree_branch(dir.path()).unwrap().is_some());
        assert_eq!(git_snapshot(dir.path()).unwrap().dirty_count(), 0);
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        let files = worktree_changes(dir.path()).unwrap();
        assert_eq!(find(&files, "a.txt").kind, ChangeKind::Update);
        assert_eq!(find(&files, "a.txt").adds, 1);
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    #[test]
    fn unsupported_scoped_changes_refuse_before_inspecting_the_workspace() {
        struct Uninspectable;

        impl AsRef<Path> for Uninspectable {
            fn as_ref(&self) -> &Path {
                panic!("an unsupported scoped reader must not inspect the path");
            }
        }

        assert_eq!(
            safe_worktree::ensure_supported().unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
        match worktree_changes_scoped(Uninspectable) {
            Err(GitError::Op(message)) => assert!(message.contains("safe opener"), "{message}"),
            other => panic!("expected an unsupported error before path access: {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_scoped_changes_refuse_every_path_spelling_without_io() {
        struct Uninspectable<'a>(&'a str);

        impl AsRef<Path> for Uninspectable<'_> {
            fn as_ref(&self) -> &Path {
                // Deliberately panic rather than passing dangerous names to
                // the OS if the entry guard ever regresses. These are guard
                // tests, NOT Windows device/reparse/pipe integration tests.
                panic!("Windows must refuse before inspecting {:?}", self.0);
            }
        }

        for workspace in [
            "",
            ".",
            "nonexistent-workspace",
            r"C:\repo\workspace",
            r"C:workspace",
            r"\workspace",
            r"..\outside",
            r"\\server\share\workspace",
            r"\\?\C:\repo\workspace",
            r"\\?\UNC\server\share\workspace",
            r"\\.\pipe\husk-scoped-preview-must-not-connect",
            r"\??\C:\repo\workspace",
            r"\Device\NamedPipe\husk-scoped-preview-must-not-connect",
            r"C:\repo\file:stream",
            r"C:\repo\file::$DATA",
            r"\\.\NUL",
            "CON",
            "CONIN$",
            "NUL.txt",
            "COM1",
            "COM¹",
            r"C:\repo\junction\child",
        ] {
            match worktree_changes_scoped(Uninspectable(workspace)) {
                Err(GitError::Op(message)) => {
                    assert!(message.contains("safe opener"), "{workspace:?}: {message}");
                }
                other => panic!("expected an unsupported error for {workspace:?}: {other:?}"),
            }
        }
    }
}
