//! Git status/diff sniffing for prompt injection.
//!
//! Contract (kernel-architecture.md §Workspace scanner):
//! `git_snapshot()` → `git status --porcelain` + `git diff --stat`,
//! truncated to ~4 KB. Implemented with `gix` (gitoxide) — pure Rust,
//! in-process, zero C linkage (Operating Principle 1).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use similar::TextDiff;
use thiserror::Error;

/// Total byte budget for the rendered snapshot block.
pub const GIT_SNAPSHOT_BUDGET: usize = 4 * 1024;

/// Files larger than this on either side skip the diff: a megabyte of source
/// renders a patch nobody reads in a side panel.
const MAX_DIFF_BYTES: u64 = 1024 * 1024;

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
    /// The diff was dropped: oversized file, or past [`MAX_CHANGED_FILES`].
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
    let repo = gix::discover(path).map_err(|_| GitError::NotARepo(path.to_path_buf()))?;
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
    let repo = gix::discover(path).map_err(|_| GitError::NotARepo(path.to_path_buf()))?;
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
pub fn worktree_changes(path: impl AsRef<Path>) -> Result<Vec<FileDiff>, GitError> {
    let path = path.as_ref();
    let repo = gix::discover(path).map_err(|_| GitError::NotARepo(path.to_path_buf()))?;
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| GitError::NotARepo(path.to_path_buf()))?;

    let mut paths = changed_paths(&repo)?;
    paths.sort();
    paths.dedup();

    // The cap is on the whole reply: keep the first N by path and mark the
    // last one, so the panel can say "and more" without a second count.
    let capped = paths.len() > MAX_CHANGED_FILES;
    paths.truncate(MAX_CHANGED_FILES);

    // Unborn HEAD has no tree: every worktree file reads as an addition.
    let head = head_tree(&repo);
    let mut files: Vec<FileDiff> = Vec::with_capacity(paths.len());
    for rel in &paths {
        let old = head.as_ref().and_then(|tree| blob_at(tree, rel));
        let Some(file) = file_diff(rel, old, &workdir.join(rel)) else {
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
fn changed_paths(repo: &gix::Repository) -> Result<Vec<String>, GitError> {
    use gix::status::index_worktree;
    use gix::status::plumbing::index_as_worktree::EntryStatus;
    use gix::status::Item;

    let mut paths: Vec<String> = Vec::new();

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
                match &change {
                    Change::Addition { location, .. }
                    | Change::Deletion { location, .. }
                    | Change::Modification { location, .. } => paths.push(location.to_string()),
                    // Both ends are paths of their own: the source is gone, the
                    // destination is new — HEAD only knows the source.
                    Change::Rewrite {
                        source_location,
                        location,
                        ..
                    } => {
                        paths.push(source_location.to_string());
                        paths.push(location.to_string());
                    }
                }
            }
            Item::IndexWorktree(iw) => match iw {
                index_worktree::Item::Modification {
                    rela_path, status, ..
                } => {
                    if !matches!(status, EntryStatus::NeedsUpdate(_)) {
                        paths.push(rela_path.to_string());
                    }
                }
                index_worktree::Item::DirectoryContents { entry, .. } => {
                    // Ignored entries stay out — the walk descends into
                    // untracked dirs but honours .gitignore first.
                    if entry.status == gix::dir::entry::Status::Untracked
                        && !entry.disk_kind.map(|k| k.is_dir()).unwrap_or(false)
                    {
                        paths.push(entry.rela_path.to_string());
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
                        } => paths.push(source_rela_path.to_string()),
                        RewriteSource::CopyFromDirectoryEntry {
                            source_dirwalk_entry,
                            ..
                        } => paths.push(source_dirwalk_entry.rela_path.to_string()),
                    }
                    paths.push(dirwalk_entry.rela_path.to_string());
                }
            },
        }
    }
    Ok(paths)
}

/// HEAD's tree, or `None` when HEAD is unborn.
fn head_tree(repo: &gix::Repository) -> Option<gix::Tree<'_>> {
    repo.head_commit().ok()?.tree().ok()
}

/// HEAD's blob at `rel`: `None` when HEAD doesn't track the path (an added
/// file) or the entry isn't a blob (a symlink or submodule).
fn blob_at(tree: &gix::Tree<'_>, rel: &str) -> Option<Vec<u8>> {
    let entry = tree.lookup_entry_by_path(rel).ok()??;
    let object = entry.object().ok()?;
    // The blob borrows the object buffer, which is dropped here — copy out.
    object.try_into_blob().ok().map(|blob| blob.data.clone())
}

/// Git's binary heuristic: a NUL byte in the head of the file.
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_LEN).any(|b| *b == 0)
}

/// Build one entry, or `None` when old and new bytes are identical (a copied
/// or racily-clean path the status walk still lists).
fn file_diff(rel: &str, old: Option<Vec<u8>>, absolute: &Path) -> Option<FileDiff> {
    let new = std::fs::read(absolute).ok();
    let kind = match (&old, &new) {
        (Some(_), None) => ChangeKind::Delete,
        (None, _) => ChangeKind::Add,
        (Some(_), Some(_)) => ChangeKind::Update,
    };
    let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
    if old == new {
        return None;
    }

    let mut file = FileDiff {
        path: rel.to_string(),
        kind,
        adds: 0,
        dels: 0,
        diff: String::new(),
        binary: false,
        truncated: false,
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
}
