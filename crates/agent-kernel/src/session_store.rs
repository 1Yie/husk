//! `session_store` — per-workspace session persistence.
//!
//! Each workspace gets `~/.local/share/husk/sessions/<ws_hash>/`:
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
    /// User-pinned sessions float to the top of `list()` ahead of recency —
    /// the sidebar's pin toggle writes this.
    #[serde(default)]
    pub pinned: bool,
    /// Usage of the last completed turn — persisted so reopening a session
    /// seeds the header meter with real numbers before the next `Usage`
    /// event arrives (history alone can't reconstruct token counts).
    #[serde(default)]
    pub usage: Option<SessionUsage>,
}

/// A session's last-known token usage, persisted inside `SessionMeta`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SessionUsage {
    pub prompt: u32,
    pub completion: u32,
    pub context_window: u32,
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
        let base = app_data_dir()
            .ok_or_else(|| std::io::Error::other("no data dir"))?
            .join("sessions")
            .join(workspace_key(workspace_root));
        std::fs::create_dir_all(&base)?;
        Ok(Self { dir: base })
    }

    /// Open an *existing* store without creating the directory — `None`
    /// when this workspace has never persisted a session. Read-only
    /// listings (the sidebar's project tree) use this so enumerating
    /// projects doesn't materialize a store dir for every workspace the
    /// user merely opened.
    pub fn open_existing(workspace_root: &Path) -> Option<Self> {
        let base = app_data_dir()?
            .join("sessions")
            .join(workspace_key(workspace_root));
        base.is_dir().then_some(Self { dir: base })
    }

    /// Path to a session's history file.
    fn history_path(&self, id: i64) -> PathBuf {
        self.dir.join(format!("{id}.jsonl"))
    }

    /// Path to a per-session scratch file (e.g. `<id>.todos.json`) — a
    /// sibling of the history file inside the app state dir, never inside
    /// the user's repository.
    pub fn state_file(&self, id: i64, name: &str) -> PathBuf {
        self.dir.join(format!("{id}.{name}"))
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    /// List sessions — pinned first, then newest activity within each tier.
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut idx = self.read_index();
        idx.sessions
            .sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
        idx.sessions
    }

    /// Toggle a session's pinned flag; returns the new value.
    pub fn set_pinned(&self, id: i64, pinned: bool) -> std::io::Result<bool> {
        let mut idx = self.read_index();
        if let Some(m) = idx.sessions.iter_mut().find(|s| s.id == id) {
            m.pinned = pinned;
            let json = serde_json::to_string_pretty(&idx)?;
            std::fs::write(self.index_path(), json)?;
            Ok(true)
        } else {
            Ok(false)
        }
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

    /// Remove a session entirely — index entry, history file, and any
    /// `<id>.*` sibling state files (todos, scratch).
    pub fn remove(&self, id: i64) -> std::io::Result<()> {
        let mut idx = self.read_index();
        idx.sessions.retain(|s| s.id != id);
        let json = serde_json::to_string_pretty(&idx)?;
        std::fs::write(self.index_path(), json)?;
        let _ = std::fs::remove_file(self.history_path(id));
        let prefix = format!("{id}.");
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(())
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

pub fn dirs_data() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// The app's data root: `~/.local/share/husk` — renamed from `agent-rs`
/// when the product got its name. A one-time `fs::rename` moves the whole
/// legacy tree (sessions/memory.db/prefs) over when the new dir is absent,
/// so existing installs keep their data.
pub fn app_data_dir() -> Option<PathBuf> {
    let base = dirs_data()?;
    let dir = base.join("husk");
    let legacy = base.join("agent-rs");
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, &dir);
    }
    Some(dir)
}

/// Recently opened workspace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentWorkspace {
    pub path: PathBuf,
    pub name: String,
    pub last_opened: u64,
}

pub fn recent_workspaces_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("recent_workspaces.json"))
}

pub fn load_recent_workspaces() -> Vec<RecentWorkspace> {
    let Some(path) = recent_workspaces_path() else { return Vec::new(); };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut list: Vec<RecentWorkspace> = serde_json::from_str(&text).unwrap_or_default();
    list.retain(|w| w.path.is_dir());
    list.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    list
}

// ---------------------------------------------------------------------------
// Workspace preferences — per-workspace UI prefs (model / thinking / permission)
// persisted next to `index.json`. Loaded at `SessionManager` boot and fed into
// `SessionConfig`; written whenever the UI changes one of the three selectors.
// ---------------------------------------------------------------------------

/// Per-workspace composer preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspacePrefs {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
}

/// Global user default preferences for fresh/new sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultPreferences {
    #[serde(default = "default_pref_permission_mode")]
    pub permission_mode: String,
    #[serde(default = "default_pref_thinking_level")]
    pub thinking_level: Option<String>,
}

fn default_pref_permission_mode() -> String {
    "auto".into()
}

fn default_pref_thinking_level() -> Option<String> {
    Some("medium".into())
}

impl Default for DefaultPreferences {
    fn default() -> Self {
        Self {
            permission_mode: default_pref_permission_mode(),
            thinking_level: default_pref_thinking_level(),
        }
    }
}

pub fn default_preferences_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("default_preferences.json"))
}

pub fn load_default_preferences() -> DefaultPreferences {
    let Some(path) = default_preferences_path() else { return DefaultPreferences::default(); };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Load the global default preferences only when the file actually exists —
/// `None` when absent or corrupt. Used as a *fallback* layer below
/// per-workspace prefs so an untouched install keeps the built-in defaults.
pub fn try_load_default_preferences() -> Option<DefaultPreferences> {
    let path = default_preferences_path()?;
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

pub fn save_default_preferences(prefs: &DefaultPreferences) -> std::io::Result<()> {
    let Some(path) = default_preferences_path() else { return Ok(()); };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(prefs)?;
    std::fs::write(path, json)
}

impl SessionStore {
    /// Path to this workspace's prefs file.
    fn prefs_path(&self) -> PathBuf {
        self.dir.join("prefs.json")
    }

    /// Load persisted prefs (empty if absent/corrupt).
    pub fn load_prefs(&self) -> WorkspacePrefs {
        std::fs::read_to_string(self.prefs_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Persist prefs (pretty JSON, overwrite).
    pub fn save_prefs(&self, prefs: &WorkspacePrefs) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(prefs)?;
        std::fs::write(self.prefs_path(), json)
    }
}

pub fn record_recent_workspace(workspace_root: &Path) {
    let Some(path) = recent_workspaces_path() else { return; };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let canon = workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
    let name = canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| canon.to_string_lossy().to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut list = load_recent_workspaces();
    list.retain(|w| w.path != canon);
    list.insert(0, RecentWorkspace {
        path: canon,
        name,
        last_opened: now,
    });
    list.truncate(15);

    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(path, json);
    }
}
