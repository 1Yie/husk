//! `HunkTracker` — file-change attribution for undo/rewind.
//!
//! Contract (kernel-architecture.md §Hunk tracking):
//!
//! - `RecordAgentWrite {path, old, new, origin}` after every successful write
//!   (built-in or plugin) → hunks keyed by turn; `origin` distinguishes
//!   `agent` vs `plugin:<id>` vs `sandbox-merge`.
//! - `HandleFileChange {path}` from a `notify` watcher attributes *external*
//!   edits — they never merge into agent undo history.
//! - Powers: per-turn "files changed" list, Undo/Rewind (reverse-apply
//!   recorded hunks), dirty-file warnings on session start.
//!
//! This module is the *tracker* (recording + query). The actor wrapper
//! (`HunkTrackerActor`) lives in kernel `channels.rs` once the `notify`
//! watcher lands — for now the engine drives it synchronously post-write.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use similar::TextDiff;

/// One recorded write — a before/after content pair plus provenance.
#[derive(Debug, Clone)]
pub struct Hunk {
    /// Turn index that produced this write.
    pub turn: u32,
    /// Who wrote it: `"agent"`, `"plugin:<id>"`, `"sandbox-merge"`.
    pub origin: String,
    /// Content before the write (`None` = file created by this write).
    pub old: Option<String>,
    /// Content after the write.
    pub new: String,
    /// Unified diff old→new (generated at record time so Undo is pure apply).
    pub unified: String,
}

/// Per-file tracked state.
#[derive(Debug, Default)]
pub struct FileState {
    /// Every write this session, oldest first.
    pub hunks: Vec<Hunk>,
    /// External edits observed by the watcher (paths only — we don't snapshot
    /// external content; it isn't ours to undo).
    pub external_changes: usize,
    /// Whether the file was dirty (git-untracked/modified) at session start.
    pub dirty_at_start: bool,
}

/// Which files the tracker watches (kernel-architecture.md §Hunk tracking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackingMode {
    /// Only agent/plugin writes are recorded.
    #[default]
    AgentOnly,
    /// Also flag files that were already dirty when the session started.
    AllDirty,
}

#[derive(Default)]
pub struct HunkTracker {
    /// path → its recorded state.
    file_states: HashMap<PathBuf, FileState>,
    /// Monotonic turn counter — the engine bumps it per user turn.
    turn_index: u32,
    mode: TrackingMode,
}

impl HunkTracker {
    pub fn new(mode: TrackingMode) -> Self {
        Self { mode, ..Default::default() }
    }

    /// Mark the start of a new user turn — subsequent writes key to it.
    pub fn begin_turn(&mut self) -> u32 {
        self.turn_index += 1;
        self.turn_index
    }

    /// The current (latest) turn index — `0` before the first `begin_turn`.
    pub fn current_turn(&self) -> u32 {
        self.turn_index
    }

    /// Record an agent/plugin write. `old = None` means file created.
    /// Generates the unified diff eagerly so Undo is a pure `similar` apply —
    /// no re-diffing at rewind time.
    pub fn record_write(
        &mut self,
        path: impl Into<PathBuf>,
        old: Option<String>,
        new: String,
        origin: impl Into<String>,
    ) {
        let path = path.into();
        let unified = unified_diff(
            old.as_deref().unwrap_or(""),
            &new,
            &path.to_string_lossy(),
        );
        self.file_states
            .entry(path)
            .or_default()
            .hunks
            .push(Hunk {
                turn: self.turn_index,
                origin: origin.into(),
                old,
                new,
                unified,
            });
    }

    /// Attribute an external (watcher-observed) change — counted, never
    /// merged into undo history (kernel-architecture.md: "don't merge them
    /// into agent undo history").
    pub fn handle_external_change(&mut self, path: impl Into<PathBuf>) {
        self.file_states
            .entry(path.into())
            .or_default()
            .external_changes += 1;
    }

    /// Files written during `turn` (the per-turn "files changed" list).
    pub fn files_in_turn(&self, turn: u32) -> Vec<&Path> {
        let mut out: Vec<&Path> = self
            .file_states
            .iter()
            .filter(|(_, st)| st.hunks.iter().any(|h| h.turn == turn))
            .map(|(p, _)| p.as_path())
            .collect();
        out.sort();
        out
    }

    /// All hunks for `path` in `turn`, oldest first.
    pub fn hunks_for(&self, path: &Path, turn: u32) -> Vec<&Hunk> {
        self.file_states
            .get(path)
            .map(|st| st.hunks.iter().filter(|h| h.turn == turn).collect())
            .unwrap_or_default()
    }

    /// The undo payload for `turn`: per-file "restore to state before this
    /// turn's first write" instructions. Applying these in reverse order
    /// reproduces the pre-turn workspace — Rewind is the same thing scoped
    /// to an earlier turn.
    pub fn undo_plan(&self, turn: u32) -> Vec<UndoOp> {
        let mut ops = Vec::new();
        for path in self.files_in_turn(turn) {
            let hunks = self.hunks_for(path, turn);
            // Restore to the *first* hunk's `old` — writes within the turn
            // are sequential, so pre-turn state = first hunk's old.
            let target = hunks.first().and_then(|h| h.old.clone());
            ops.push(UndoOp {
                path: path.to_path_buf(),
                restore_to: target, // None → file was created this turn → delete it
            });
        }
        ops
    }

    /// Dirty-file warnings for session start (`AllDirty` mode) — caller seeds
    /// this by marking files it knows were dirty.
    pub fn mark_dirty_at_start(&mut self, path: impl Into<PathBuf>) {
        if self.mode == TrackingMode::AllDirty {
            self.file_states
                .entry(path.into())
                .or_default()
                .dirty_at_start = true;
        }
    }

    pub fn dirty_at_start(&self) -> Vec<&Path> {
        self.file_states
            .iter()
            .filter(|(_, st)| st.dirty_at_start)
            .map(|(p, _)| p.as_path())
            .collect()
    }

    pub fn total_hunks(&self) -> usize {
        self.file_states.values().map(|s| s.hunks.len()).sum()
    }
}

/// One step of an undo plan: restore `path` to `restore_to` content, or
/// delete the file when `restore_to` is `None` (it was created this turn).
#[derive(Debug, Clone, PartialEq)]
pub struct UndoOp {
    pub path: PathBuf,
    /// `Some(content)` → write it; `None` → delete the file.
    pub restore_to: Option<String>,
}

/// `similar`-generated unified diff — stored eagerly so Undo doesn't re-diff.
fn unified_diff(old: &str, new: &str, path: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_groups_by_turn() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.record_write("b.rs", None, "new".into(), "agent");
        t.begin_turn();
        t.record_write("a.rs", Some("v2".into()), "v3".into(), "agent");

        assert_eq!(t.files_in_turn(1).len(), 2);
        assert_eq!(t.files_in_turn(2), vec![Path::new("a.rs")]);
        assert_eq!(t.total_hunks(), 3);
    }

    #[test]
    fn undo_plan_restores_pre_turn_state() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.record_write("a.rs", Some("v2".into()), "v3".into(), "agent");
        t.record_write("b.rs", None, "created".into(), "agent");

        let plan = t.undo_plan(1);
        let a = plan.iter().find(|o| o.path == Path::new("a.rs")).unwrap();
        let b = plan.iter().find(|o| o.path == Path::new("b.rs")).unwrap();
        assert_eq!(a.restore_to.as_deref(), Some("v1")); // first hunk's old
        assert_eq!(b.restore_to, None);                  // created → delete
    }

    #[test]
    fn external_changes_dont_pollute_undo() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.handle_external_change("b.rs"); // external — not ours to undo

        let plan = t.undo_plan(1);
        assert_eq!(plan.len(), 1); // only a.rs
        assert_eq!(plan[0].path, Path::new("a.rs"));
    }
}
