//! Git status/diff sniffing for prompt injection.
//!
//! Contract (kernel-architecture.md §Workspace scanner):
//! `git_snapshot()` → `git status --porcelain` + `git diff --stat`,
//! truncated to ~4 KB. Implemented with `gix` (gitoxide) — pure Rust,
//! in-process, zero C linkage (Operating Principle 1).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Total byte budget for the rendered snapshot block.
pub const GIT_SNAPSHOT_BUDGET: usize = 4 * 1024;

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
fn collect_status_and_stat(
    repo: &gix::Repository,
) -> Result<(Vec<String>, Vec<String>), GitError> {
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
                        (
                            c,
                            format!("{} -> {}", source_location, location),
                        )
                    }
                };
                status_lines.push(format!("{code}  {path}"));
                stat.push(path);
            }
            Item::IndexWorktree(iw) => match iw {
                index_worktree::Item::Modification {
                    rela_path, status: es, ..
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
            snap.status.iter().any(|l| l.starts_with(" M") && l.contains("a.txt")),
            "status: {:?}",
            snap.status
        );
        assert!(
            snap.status.iter().any(|l| l.starts_with("??") && l.contains("new.rs")),
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
            snap.status.iter().any(|l| l.starts_with("M ") && l.contains("a.txt")),
            "status: {:?}",
            snap.status
        );
    }

    #[test]
    fn budget_enforced() {
        let dir = init_repo();
        for i in 0..500 {
            fs::write(
                dir.path()
                    .join(format!("very_long_filename_to_bloat_the_status_block_{i:04}.txt")),
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
}
