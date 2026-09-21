//! Workspace file-tree scanner.
//!
//! Contract (kernel-architecture.md §Workspace scanner):
//! `WorkspaceScanner::build_file_tree(root, max_depth=3)` → `Vec<String>`
//! relative paths; `ignore::WalkBuilder` with `.hidden(true).git_ignore(true)
//! .git_exclude(true)`; cap ~2000 entries with a fold marker.
//!
//! Everything runs in-process — no `find`/`ls` subprocess (Operating
//! Principle 1: single static binary, no shelling out unless designed).

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use thiserror::Error;

/// Hard cap on entries returned by a scan. Anything beyond this collapses
/// into a single fold marker so the LLM prompt stays inside budget.
pub const MAX_TREE_ENTRIES: usize = 2_000;

/// Hard cap on the flat file index used by the `@` completion picker.
/// Larger than [`MAX_TREE_ENTRIES`] — the picker needs deep paths the
/// prompt tree folds away — but still bounded for huge monorepos.
pub const MAX_INDEX_ENTRIES: usize = 20_000;

/// Default directory depth for the skeleton tree handed to the model.
pub const DEFAULT_MAX_DEPTH: usize = 3;

/// Marker appended when the tree hits [`MAX_TREE_ENTRIES`].
pub const FOLD_MARKER: &str = "... [truncated]";

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace root does not exist or is not a directory: {0}")]
    InvalidRoot(PathBuf),

    #[error("workspace scan failed: {0}")]
    Scan(#[from] ignore::Error),
}

/// Result of a workspace scan. `entries` are workspace-relative, `/`-separated
/// paths (directories carry a trailing `/`). `truncated` is true when the
/// entry cap dropped real files.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceTree {
    pub root: PathBuf,
    pub entries: Vec<String>,
    pub truncated: bool,
}

impl WorkspaceTree {
    /// Render the tree as the `{{WORKSPACE_TREE}}` substitution block for the
    /// kernel system prompt — one relative path per line, fold marker last.
    pub fn to_prompt_block(&self) -> String {
        let mut out = String::with_capacity(self.entries.len() * 24);
        for e in &self.entries {
            out.push_str(e);
            out.push('\n');
        }
        if self.truncated {
            out.push_str(FOLD_MARKER);
            out.push('\n');
        }
        out
    }
}

/// Stateless scanner — configuration lives on the builder call, matching the
/// spec's free-function-shaped contract.
pub struct WorkspaceScanner;

impl WorkspaceScanner {
    /// Build the skeleton file tree for `root`, relative paths only.
    ///
    /// * Respects `.gitignore`, `.git/info/exclude`, and global git excludes.
    /// * Skips hidden files/dirs.
    /// * Descends at most `max_depth` levels below root.
    /// * Stops after [`MAX_TREE_ENTRIES`] entries and marks the result
    ///   truncated rather than streaming an unbounded listing into the model.
    pub fn build_file_tree(
        root: impl AsRef<Path>,
        max_depth: usize,
    ) -> Result<WorkspaceTree, WorkspaceError> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(WorkspaceError::InvalidRoot(root.to_path_buf()));
        }
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false) // .gitignore still applies outside git repos
            .max_depth(Some(max_depth.saturating_add(1))) // walker counts root as depth 0
            .sort_by_file_name(|a, b| a.cmp(b))
            .follow_links(false);

        let mut tree = WorkspaceTree {
            root: root.clone(),
            ..Default::default()
        };

        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(%err, "workspace walk entry error, skipping");
                    continue;
                }
            };

            // Skip the root itself (depth 0).
            if entry.depth() == 0 {
                continue;
            }

            if tree.entries.len() >= MAX_TREE_ENTRIES {
                tree.truncated = true;
                break;
            }

            let rel = match entry.path().strip_prefix(&root) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let mut s = rel.to_string_lossy().replace('\\', "/");
            if s.is_empty() {
                continue;
            }
            if entry.file_type().is_some_and(|ft| ft.is_dir()) && !s.ends_with('/') {
                s.push('/');
            }
            tree.entries.push(s);
        }

        Ok(tree)
    }

    /// Spec-default scan: `max_depth = 3`.
    pub fn build_skeleton(root: impl AsRef<Path>) -> Result<WorkspaceTree, WorkspaceError> {
        Self::build_file_tree(root, DEFAULT_MAX_DEPTH)
    }

    /// Flat index of every non-ignored FILE in the workspace — `/`-separated
    /// relative paths, no depth cap, sorted by file name. Feeds the `@`
    /// mention picker (directories are omitted — it completes files, and
    /// the kernel inlines content on submit). `cap` bounds the result so a
    /// monorepo can't stall the composer; `0` means [`MAX_INDEX_ENTRIES`].
    pub fn build_file_index(
        root: impl AsRef<Path>,
        cap: usize,
    ) -> Result<Vec<String>, WorkspaceError> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(WorkspaceError::InvalidRoot(root.to_path_buf()));
        }
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let cap = if cap == 0 { MAX_INDEX_ENTRIES } else { cap };

        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false)
            .sort_by_file_name(|a, b| a.cmp(b))
            .follow_links(false);

        let mut out = Vec::new();
        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(%err, "workspace walk entry error, skipping");
                    continue;
                }
            };
            if entry.depth() == 0 || entry.file_type().is_some_and(|ft| ft.is_dir()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&root) else {
                continue;
            };
            let s = rel.to_string_lossy().replace('\\', "/");
            if s.is_empty() {
                continue;
            }
            out.push(s);
            if out.len() >= cap {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn invalid_root() {
        assert!(matches!(
            WorkspaceScanner::build_skeleton("/nonexistent/path/xyz"),
            Err(WorkspaceError::InvalidRoot(_))
        ));
    }

    #[test]
    fn respects_gitignore_and_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join(".gitignore"), "target/\nsecret.txt\n").unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target/out.bin"), "x").unwrap();
        fs::write(root.join("secret.txt"), "s").unwrap();
        fs::write(root.join(".hidden"), "h").unwrap();
        fs::create_dir_all(root.join(".hidden_dir")).unwrap();
        fs::write(root.join(".hidden_dir/x"), "h").unwrap();

        // require_git(false) makes .gitignore apply even outside a repo.
        let tree = WorkspaceScanner::build_skeleton(root).unwrap();
        assert!(tree.entries.iter().any(|e| e == "src/"));
        assert!(tree.entries.iter().any(|e| e == "src/main.rs"));
        assert!(!tree.entries.iter().any(|e| e.contains("secret.txt")));
        assert!(!tree.entries.iter().any(|e| e.contains("target")));
        assert!(!tree.entries.iter().any(|e| e.contains(".hidden")));
    }

    #[test]
    fn max_depth_folds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b/c/d")).unwrap();
        fs::write(root.join("a/b/c/d/deep.txt"), "x").unwrap();
        fs::write(root.join("a/top.txt"), "x").unwrap();

        // max_depth=2 → walker max_depth=3 (root is depth 0), so entries at
        // depth ≤3 appear but aren't descended: a/b/c/ is listed, its
        // contents (depth 4+) are folded away.
        let tree = WorkspaceScanner::build_file_tree(root, 2).unwrap();
        assert!(tree.entries.iter().any(|e| e == "a/top.txt"));
        assert!(tree.entries.iter().any(|e| e == "a/b/c/"));
        assert!(!tree.entries.iter().any(|e| e.contains("deep.txt")));
    }

    #[test]
    fn entry_cap_sets_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..MAX_TREE_ENTRIES + 50 {
            fs::write(root.join(format!("f{i:05}.txt")), "x").unwrap();
        }
        let tree = WorkspaceScanner::build_skeleton(root).unwrap();
        assert_eq!(tree.entries.len(), MAX_TREE_ENTRIES);
        assert!(tree.truncated);
        assert!(tree.to_prompt_block().ends_with("... [truncated]\n"));
    }

    #[test]
    fn file_index_lists_deep_files_and_skips_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b/c/d")).unwrap();
        fs::write(root.join("a/b/c/d/deep.txt"), "x").unwrap();
        fs::write(root.join("top.rs"), "fn main() {}").unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::write(root.join("ignored/x.txt"), "x").unwrap();

        // No depth cap — the picker must reach deep paths the prompt tree
        // folds away.
        let idx = WorkspaceScanner::build_file_index(root, 0).unwrap();
        assert!(idx.iter().any(|e| e == "a/b/c/d/deep.txt"));
        assert!(idx.iter().any(|e| e == "top.rs"));
        assert!(!idx.iter().any(|e| e.ends_with('/')));
        assert!(!idx.iter().any(|e| e.contains("ignored")));
    }

    #[test]
    fn file_index_respects_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..10 {
            fs::write(root.join(format!("f{i}.txt")), "x").unwrap();
        }
        let idx = WorkspaceScanner::build_file_index(root, 4).unwrap();
        assert_eq!(idx.len(), 4);
    }
}
