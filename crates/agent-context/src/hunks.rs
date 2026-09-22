//! `HunkTracker` — file-change attribution for undo/rewind.
//!
//! `record_write` runs after every successful write, keyed by turn, with `origin`
//! distinguishing `agent` / `plugin:<id>` / `sandbox-merge`; a watcher-observed edit
//! goes through `handle_external_change` and never merges into agent undo history.
//! Powers per-turn "files changed", Undo/Rewind and dirty-file warnings.
//!
//! `Hunk.old`/`new` are `Vec<u8>`, so binary writes are undoable, and `undo_plan`
//! refuses to overwrite a file modified externally after the agent's last write.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use similar::TextDiff;

/// One recorded write — a before/after content pair plus provenance.
///
/// `old`/`new` are raw bytes (`Vec<u8>`), not `String`, so binary and
/// non-UTF-8 files are recorded faithfully and can be restored by Undo.
/// `unified` is only produced for UTF-8 content (it is display-only).
#[derive(Debug, Clone)]
pub struct Hunk {
    /// Turn index that produced this write.
    pub turn: u32,
    /// Who wrote it: `"agent"`, `"plugin:<id>"`, `"sandbox-merge"`.
    pub origin: String,
    /// Content before the write (`None` = file created by this write).
    /// `Some(vec![])`+`content_dropped` = was present but evicted for size.
    pub old: Option<Vec<u8>>,
    /// Content after the write. Empty + `content_dropped` = evicted for size.
    pub new: Vec<u8>,
    /// Unified diff old→new, `""` for non-UTF-8 content (display-only).
    pub unified: String,
    /// True when `old`/`new` were dropped to keep the tracker under
    /// `MAX_HUNK_CONTENT` — the file can't be byte-restored from this hunk
    /// (undo_plan treats it like an external change and skips/refuses).
    pub content_dropped: bool,
}

/// Per-file tracked state.
#[derive(Debug, Default)]
pub struct FileState {
    /// Every write this session, oldest first.
    pub hunks: Vec<Hunk>,
    /// External edits observed by the watcher (paths only — we don't snapshot
    /// external content; it isn't ours to undo).
    pub external_changes: usize,
    /// Index (into `hunks`) of the agent's last write *before* the most
    /// recent external change. `None` = no external change after any write.
    /// Used by `undo_plan` to detect "undo would clobber a user edit".
    pub external_after_last_write: bool,
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

/// Max bytes retained for a single hunk's `old`/`new` content — a file
/// bigger than this is tracked by path+turn only (its content is too large
/// to keep for undo; restoring would need a full re-read anyway). Prevents
/// one huge write from bloating the tracker (P2).
const MAX_HUNK_CONTENT: usize = 2 * 1024 * 1024; // 2 MiB

/// Max total hunk records kept across all files — oldest evicted first.
/// A long session of many small writes stays bounded (P2).
const MAX_TOTAL_HUNKS: usize = 10_000;

#[derive(Default)]
pub struct HunkTracker {
    /// path → its recorded state.
    file_states: HashMap<PathBuf, FileState>,
    /// Monotonic turn counter — the engine bumps it per user turn.
    turn_index: u32,
    mode: TrackingMode,
    /// Total hunk count across `file_states` (for the global cap).
    total: usize,
}

/// Why an undo was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoError {
    /// The file was modified externally after the agent's last write —
    /// restoring would silently destroy the user's own edits.
    ExternalChange { path: PathBuf },
}

impl std::fmt::Display for UndoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UndoError::ExternalChange { path } => write!(
                f,
                "refusing to undo `{}` — it was modified outside the agent \
                 after the last recorded write (would clobber user edits)",
                path.display()
            ),
        }
    }
}
impl std::error::Error for UndoError {}

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
    /// Generates the unified diff eagerly for UTF-8 content so Undo is a
    /// pure restore — no re-diffing at rewind time.
    ///
    /// Bounded: a hunk whose `old`/`new` exceeds `MAX_HUNK_CONTENT` has its
    /// content dropped (path+turn still recorded, undo skips it); when the
    /// total hunk count passes `MAX_TOTAL_HUNKS` the oldest records are
    /// evicted first.
    pub fn record_write(
        &mut self,
        path: impl Into<PathBuf>,
        old: Option<Vec<u8>>,
        new: Vec<u8>,
        origin: impl Into<String>,
    ) {
        let path = path.into();
        let unified = match (&old, std::str::from_utf8(&new)) {
            (Some(o), Ok(n)) => std::str::from_utf8(o)
                .map(|o| unified_diff(o, n, &path.to_string_lossy()))
                .unwrap_or_default(),
            (None, Ok(n)) => unified_diff("", n, &path.to_string_lossy()),
            _ => String::new(), // binary — no line diff
        };

        // Content over the cap is dropped — a huge file is tracked by path
        // only; it can't be byte-restored anyway.
        let (old, new, content_dropped) = {
            let too_big = old.as_ref().map(|o| o.len()).unwrap_or(0) > MAX_HUNK_CONTENT
                || new.len() > MAX_HUNK_CONTENT;
            if too_big {
                (old.map(|_| Vec::new()), Vec::new(), true)
            } else {
                (old, new, false)
            }
        };

        let st = self.file_states.entry(path).or_default();
        // A fresh agent write re-baselines the "external after last write"
        // flag — the newest hunk is now the restore point we trust.
        st.external_after_last_write = false;
        st.hunks.push(Hunk {
            turn: self.turn_index,
            origin: origin.into(),
            old,
            new,
            unified,
            content_dropped,
        });
        self.total += 1;
        self.evict_if_needed();
    }

    /// Drop the oldest hunks once `total` exceeds `MAX_TOTAL_HUNKS`.
    fn evict_if_needed(&mut self) {
        while self.total > MAX_TOTAL_HUNKS {
            // Evict the globally oldest hunk (lowest turn) first.
            if let Some((_, st)) = self
                .file_states
                .iter_mut()
                .filter(|(_, s)| !s.hunks.is_empty())
                .min_by_key(|(_, s)| s.hunks[0].turn)
            {
                st.hunks.remove(0);
                self.total -= 1;
            } else {
                break;
            }
        }
    }

    /// UTF-8 convenience wrapper over [`Self::record_write`].
    pub fn record_write_str(
        &mut self,
        path: impl Into<PathBuf>,
        old: Option<String>,
        new: String,
        origin: impl Into<String>,
    ) {
        self.record_write(
            path,
            old.map(String::into_bytes),
            new.into_bytes(),
            origin,
        );
    }

    /// Attribute an external (watcher-observed) change — counted, never
    /// merged into undo history. Marks `external_after_last_write` when the
    /// file has recorded agent hunks, so a later undo won't clobber it.
    pub fn handle_external_change(&mut self, path: impl Into<PathBuf>) {
        let st = self.file_states.entry(path.into()).or_default();
        st.external_changes += 1;
        if !st.hunks.is_empty() {
            st.external_after_last_write = true;
        }
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
    /// turn's first write" instructions. Rewind is the same thing scoped to
    /// an earlier turn.
    ///
    /// Returns `Err(UndoError::ExternalChange)` if any target file was
    /// modified externally *after* the agent's last recorded write —
    /// undoing would silently destroy that user edit, so we refuse and
    /// surface the path instead of clobbering it.
    pub fn undo_plan(&self, turn: u32) -> Result<Vec<UndoOp>, UndoError> {
        let mut ops = Vec::new();
        for path in self.files_in_turn(turn) {
            let st = self.file_states.get(path).expect("files_in_turn yields known paths");
            // Refuse when the file can't be safely restored: externally
            // modified after our last write, or hunk content evicted for
            // size (no bytes to restore).
            if st.external_after_last_write
                || st.hunks.iter().any(|h| h.turn == turn && h.content_dropped)
            {
                return Err(UndoError::ExternalChange {
                    path: path.to_path_buf(),
                });
            }
            let hunks = self.hunks_for(path, turn);
            // Restore to the *first* hunk's `old` — writes within the turn
            // are sequential, so pre-turn state = first hunk's old.
            let target = hunks.first().and_then(|h| h.old.clone());
            ops.push(UndoOp {
                path: path.to_path_buf(),
                restore_to: target, // None → file was created this turn → delete it
            });
        }
        Ok(ops)
    }

    /// Non-failing variant: skips unrestorable files and returns them in the
    /// second tuple slot.
    pub fn undo_plan_partial(&self, turn: u32) -> (Vec<UndoOp>, Vec<PathBuf>) {
        let mut ops = Vec::new();
        let mut skipped = Vec::new();
        for path in self.files_in_turn(turn) {
            let st = self.file_states.get(path).expect("files_in_turn yields known paths");
            if st.external_after_last_write
                || st.hunks.iter().any(|h| h.turn == turn && h.content_dropped)
            {
                skipped.push(path.to_path_buf());
                continue;
            }
            let hunks = self.hunks_for(path, turn);
            let target = hunks.first().and_then(|h| h.old.clone());
            ops.push(UndoOp {
                path: path.to_path_buf(),
                restore_to: target,
            });
        }
        (ops, skipped)
    }

    /// Seed dirty-file warnings for session start (`AllDirty` mode only).
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

/// One step of an undo plan: restore `path` to `restore_to` bytes, or
/// delete the file when `restore_to` is `None` (it was created this turn).
#[derive(Debug, Clone, PartialEq)]
pub struct UndoOp {
    pub path: PathBuf,
    /// `Some(bytes)` → write them; `None` → delete the file.
    /// Raw bytes so binary restores round-trip losslessly.
    pub restore_to: Option<Vec<u8>>,
}

/// `similar`-generated unified diff — stored eagerly so Undo doesn't re-diff.
/// Only called on UTF-8 content by `record_write`.
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
        t.record_write_str("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.record_write_str("b.rs", None, "new".into(), "agent");
        t.begin_turn();
        t.record_write_str("a.rs", Some("v2".into()), "v3".into(), "agent");

        assert_eq!(t.files_in_turn(1).len(), 2);
        assert_eq!(t.files_in_turn(2), vec![Path::new("a.rs")]);
        assert_eq!(t.total_hunks(), 3);
    }

    #[test]
    fn undo_plan_restores_pre_turn_state() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write_str("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.record_write_str("a.rs", Some("v2".into()), "v3".into(), "agent");
        t.record_write_str("b.rs", None, "created".into(), "agent");

        let plan = t.undo_plan(1).unwrap();
        let a = plan.iter().find(|o| o.path == Path::new("a.rs")).unwrap();
        let b = plan.iter().find(|o| o.path == Path::new("b.rs")).unwrap();
        assert_eq!(a.restore_to.as_deref(), Some(b"v1".as_slice()));
        assert_eq!(b.restore_to, None); // created → delete
    }

    #[test]
    fn external_changes_dont_pollute_undo() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write_str("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.handle_external_change("b.rs");

        let plan = t.undo_plan(1).unwrap();
        assert_eq!(plan.len(), 1); // only a.rs
        assert_eq!(plan[0].path, Path::new("a.rs"));
    }

    #[test]
    fn undo_refuses_to_clobber_external_edit() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write_str("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.handle_external_change("a.rs"); // user edit after the agent write

        let err = t.undo_plan(1).unwrap_err();
        assert_eq!(
            err,
            UndoError::ExternalChange {
                path: PathBuf::from("a.rs")
            }
        );
    }

    #[test]
    fn undo_partial_skips_external_but_restores_rest() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write_str("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.record_write_str("b.rs", Some("x".into()), "y".into(), "agent");
        t.handle_external_change("a.rs");

        let (ops, skipped) = t.undo_plan_partial(1);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path, Path::new("b.rs"));
        assert_eq!(skipped, vec![PathBuf::from("a.rs")]);
    }

    #[test]
    fn binary_writes_are_recorded() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        let old = vec![0u8, 159, 146, 150]; // non-UTF-8
        let new = vec![0u8, 159, 146, 151];
        t.record_write("bin.dat", Some(old.clone()), new.clone(), "agent");

        let plan = t.undo_plan(1).unwrap();
        assert_eq!(plan[0].restore_to, Some(old));
        // No line-diff produced for binary content.
        assert!(t.hunks_for(Path::new("bin.dat"), 1)[0].unified.is_empty());
    }

    #[test]
    fn fresh_agent_write_rebaselines_external_flag() {
        let mut t = HunkTracker::new(TrackingMode::AgentOnly);
        t.begin_turn();
        t.record_write_str("a.rs", Some("v1".into()), "v2".into(), "agent");
        t.handle_external_change("a.rs");
        // A fresh write re-baselines: undo to the newest hunk's `old` is
        // permitted again.
        t.record_write_str("a.rs", Some("vX".into()), "v3".into(), "agent");
        let plan = t.undo_plan(1).unwrap();
        assert_eq!(plan[0].restore_to.as_deref(), Some(b"v1".as_slice()));
    }
}
