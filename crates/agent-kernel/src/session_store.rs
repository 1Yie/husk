//! `session_store` — per-workspace session persistence.
//!
//! Each workspace gets `~/.local/share/agent-rs/sessions/<ws_hash>/`:
//!   - `index.json`            — session metadata (title/preview/updated_at)
//!   - `<id>.jsonl`            — history snapshots, one `Vec<ChatMessage>` per line
//!                             (last line = latest state; crash-safe append)
//!
//! Design (contract §session persistence):
//!   - Single-writer: the owning `SessionActor` dumps its `history` on a
//!     turn boundary; the store never locks across turns.
//!   - Resume = `SessionActor::resume(cfg, history)` — the LLM context is
//!     exactly the prior history; workspace tree + memory refresh at spawn.
//!   - Background turns keep their actor alive — the store only snapshots
//!     at idle boundaries (after a turn finishes), so a running session's
//!     file is always the last *completed* state.

use std::path::{Path, PathBuf};

use agent_llm::types::ChatMessage;
use serde::{Deserialize, Serialize};

/// One session's sidebar metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: i64,
    pub title: String,
    /// Preview of the last agent reply (or "running…" mid-turn).
    pub preview: String,
    /// Unix seconds of the last activity.
    pub updated_at: u64,
}

/// The on-disk session index for one workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    pub sessions: Vec<SessionMeta>,
}

/// Per-workspace session store.
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    /// Open (creating) the store for a workspace root.
    pub fn open(workspace_root: &Path) -> std::io::Result<Self> {
        let base = dirs_data()
            .ok_or_else(|| std::io::Error::other("no data dir"))?
            .join("agent-rs")
            .join("sessions")
            .join(workspace_key(workspace_root));
        std::fs::create_dir_all(&base)?;
        Ok(Self { dir: base })
    }

    /// Path to a session's history file.
    fn history_path(&self, id: i64) -> PathBuf {
        self.dir.join(format!("{id}.jsonl"))
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    /// List sessions, newest first.
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut idx = self.read_index();
        idx.sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        idx.sessions
    }

    /// Load the latest history snapshot for a session.
    pub fn load_history(&self, id: i64) -> Option<Vec<ChatMessage>> {
        let text = std::fs::read_to_string(self.history_path(id)).ok()?;
        let last = text.lines().rev().find(|l| !l.trim().is_empty())?;
        serde_json::from_str(last).ok()
    }

    /// Append a full history snapshot (one line). Called at turn boundaries.
    pub fn snapshot(&self, id: i64, history: &[ChatMessage]) -> std::io::Result<()> {
        use std::io::Write;
        let line = serde_json::to_string(history)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.history_path(id))?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        Ok(())
    }

    /// Update sidebar metadata for a session (insert or replace by id).
    pub fn upsert_meta(&self, meta: SessionMeta) -> std::io::Result<()> {
        let mut idx = self.read_index();
        match idx.sessions.iter_mut().find(|s| s.id == meta.id) {
            Some(s) => *s = meta,
            None => idx.sessions.push(meta),
        }
        let json = serde_json::to_string_pretty(&idx)?;
        std::fs::write(self.index_path(), json)
    }

    /// Allocate a fresh session id (max existing + 1, starting at 1).
    pub fn next_id(&self) -> i64 {
        self.read_index()
            .sessions
            .iter()
            .map(|s| s.id)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn read_index(&self) -> SessionIndex {
        std::fs::read_to_string(self.index_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
}

/// Stable per-workspace key — xxh3 of the canonical root.
fn workspace_key(root: &Path) -> String {
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let h = xxhash_rust::xxh3::xxh3_64(canon.to_string_lossy().as_bytes());
    format!("{h:016x}")
}

fn dirs_data() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}
