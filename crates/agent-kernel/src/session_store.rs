//! `session_store` — per-workspace session persistence.
//!
//! All state lives in a single redb database at
//! `~/.local/share/husk/sessions.db`. It replaces the old
//! `sessions/<ws_hash>/` tree — `index.json`, append-only `<id>.jsonl`,
//! `prefs.json`, and `<id>.todos.json` siblings — which is imported once
//! at first open and then renamed aside (`sessions.legacy/`, `*.json.bak`).
//!
//! Tables:
//!   - `metas/<ws>/<id>`        — `SessionMeta` JSON (sidebar index rows)
//!   - `history/<ws>/<id>`      — latest `Vec<ChatMessage>` snapshot,
//!                                OVERWRITTEN per turn. The .jsonl file grew
//!                                with `turns × history` because every line
//!                                was a full copy; an atomic `put` keeps one
//!                                copy, so rotation and torn-tail recovery
//!                                are unnecessary — a crash costs at most
//!                                the uncommitted turn, never earlier state.
//!   - `state/<ws>/<id>/<name>` — per-session scratch blobs (todos, …)
//!   - `prefs/<ws>`             — `WorkspacePrefs` JSON
//!   - `workspaces/<ws>`        — `{root, last_active}`
//!   - `globals/<name>`         — `recent_workspaces` | `default_preferences`
//!                                | `appearance`
//!
//! The owning `SessionActor` writes on turn boundaries only (single writer,
//! never mid-turn); redb commits are atomic, so readers always see the last
//! completed state — never a half-written record.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use agent_llm::types::ChatMessage;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const DB_FILE: &str = "sessions.db";

const METAS: TableDefinition<&str, &[u8]> = TableDefinition::new("metas");
const HISTORY: TableDefinition<&str, &[u8]> = TableDefinition::new("history");
const STATE: TableDefinition<&str, &[u8]> = TableDefinition::new("state");
const PREFS: TableDefinition<&str, &[u8]> = TableDefinition::new("prefs");
const WORKSPACES: TableDefinition<&str, &[u8]> = TableDefinition::new("workspaces");
const GLOBALS: TableDefinition<&str, &[u8]> = TableDefinition::new("globals");

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

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
    /// Model that produced `usage` — the settings 统计 pane groups spend by
    /// model, which the token counts alone cannot tell apart. Doubles as the
    /// session's persisted model selection: `spawn_actor` restores it so a
    /// reopened session keeps the model it was last run with.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// Per-session composer settings — written by the actor the moment a
    /// `SetModel`/`SetPermissionMode`/`SetAgentMode`/`SetThinkingLevel`
    /// command lands, so a session's choices are its own (not a workspace-wide
    /// default that leaks across sessions). `None` = "fall back to the
    /// workspace default at spawn".
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub agent_mode: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    /// Parked follow-up prompts the composer queued mid-turn — session
    /// state like `history`, so a switched-away-and-back or reopened
    /// session still has them. Written on every queue mutation; restored
    /// by `spawn_actor`.
    #[serde(default)]
    pub queued_prompts: Vec<String>,
}

/// A session's last-known token usage, persisted inside `SessionMeta`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SessionUsage {
    pub prompt: u32,
    pub completion: u32,
    pub context_window: u32,
    /// Prompt tokens served from the provider cache — `prompt - cached`
    /// is the uncached/billed share. `default` keeps old meta files valid.
    #[serde(default)]
    pub cached: u32,
}

/// The legacy on-disk session index (`sessions/<ws>/index.json`) — kept as
/// the migration's deserialization shape only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    /// The session the user last had open — restored on next launch
    /// instead of defaulting to the most recently updated one.
    #[serde(default)]
    pub last_active: Option<i64>,
    pub sessions: Vec<SessionMeta>,
}

/// Per-workspace row in the `workspaces` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkspaceRow {
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    last_active: Option<i64>,
}

/// Per-workspace session store — one row namespace (`<ws>/…`) inside the
/// shared `sessions.db`.
pub struct SessionStore {
    ws: String,
    db: Arc<Database>,
}

impl SessionStore {
    /// Open (creating) the store for a workspace root.
    pub fn open(workspace_root: &Path) -> std::io::Result<Self> {
        let db = global_db()
            .or_else(|| {
                // Test builds must not depend on (or lock-fight with the
                // running app over) the real `sessions.db` — fall back to a
                // per-process temp db instead. `#[cfg(test)]` keeps this out
                // of the shipped binary, where "no data dir" is a real error.
                #[cfg(test)]
                {
                    static TMP_DB: OnceLock<Option<Arc<Database>>> = OnceLock::new();
                    return TMP_DB
                        .get_or_init(|| {
                            let path = std::env::temp_dir().join(format!(
                                "husk-test-{}-sessions.db",
                                std::process::id()
                            ));
                            open_db(&path).ok().map(Arc::new)
                        })
                        .clone();
                }
                #[cfg(not(test))]
                None
            })
            .ok_or_else(|| std::io::Error::other("no data dir"))?;
        Self::open_with(workspace_root, db)
    }

    /// Open on a caller-supplied database — the injection seam tests use to
    /// keep stores on an isolated temp db instead of the app-wide one.
    pub fn open_with(workspace_root: &Path, db: Arc<Database>) -> std::io::Result<Self> {
        let store = Self {
            ws: workspace_key(workspace_root),
            db,
        };
        store.ensure_workspace(workspace_root)?;
        Ok(store)
    }

    /// Open on a database at an explicit path — a convenience over
    /// `open_with` for tests/tools that want a self-contained store.
    pub fn open_at(workspace_root: &Path, db_path: &Path) -> std::io::Result<Self> {
        let db = Arc::new(open_db(db_path)?);
        Self::open_with(workspace_root, db)
    }

    /// Open an *existing* store without creating the workspace row — `None`
    /// when this workspace has never persisted anything. Read-only listings
    /// (the sidebar's project tree) use this so enumerating projects doesn't
    /// materialize rows for every workspace the user merely opened.
    pub fn open_existing(workspace_root: &Path) -> Option<Self> {
        let db = global_db()?;
        let store = Self {
            ws: workspace_key(workspace_root),
            db,
        };
        store.workspace_row().ok()??;
        store.record_root(workspace_root);
        Some(store)
    }

    /// Every workspace store on record (`workspaces` table keys). The 统计
    /// pane aggregates across ALL of them: recents only remembers the
    /// projects the user opened through this install, so anything older — or
    /// any project whose recent entry was pruned — was invisible in the
    /// totals.
    pub fn all_workspaces() -> Vec<Self> {
        let Some(db) = global_db() else {
            return Vec::new();
        };
        let Ok(rtx) = db.begin_read() else {
            return Vec::new();
        };
        let Ok(t) = rtx.open_table(WORKSPACES) else {
            return Vec::new();
        };
        let Ok(iter) = t.iter() else {
            return Vec::new();
        };
        iter.flatten()
            .map(|(k, _)| Self {
                ws: k.value().to_string(),
                db: db.clone(),
            })
            .collect()
    }

    /// This store's workspace key hash.
    pub fn dir_name(&self) -> Option<String> {
        Some(self.ws.clone())
    }

    fn workspace_row(&self) -> std::io::Result<Option<WorkspaceRow>> {
        let rtx = self.db.begin_read().map_err(io_err)?;
        let t = rtx.open_table(WORKSPACES).map_err(io_err)?;
        match t.get(self.ws.as_str()).map_err(io_err)? {
            Some(g) => Ok(serde_json::from_slice(g.value()).ok()),
            None => Ok(None),
        }
    }

    fn put_workspace_row(&self, row: &WorkspaceRow) -> std::io::Result<()> {
        let json = serde_json::to_vec(row).map_err(io_err)?;
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(WORKSPACES).map_err(io_err)?;
            t.insert(self.ws.as_str(), json.as_slice())
                .map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)
    }

    /// Create the workspace row if absent; also records the root (write-once).
    fn ensure_workspace(&self, root: &Path) -> std::io::Result<()> {
        let mut row = self.workspace_row()?.unwrap_or_default();
        if row.root.is_some() {
            return Ok(()); // row exists with root recorded — nothing to write
        }
        row.root = Some(root.to_string_lossy().into_owned());
        self.put_workspace_row(&row)
    }

    /// The workspace root this store belongs to, if it was ever recorded.
    pub fn recorded_root(&self) -> Option<PathBuf> {
        self.workspace_row()
            .ok()
            .flatten()
            .and_then(|r| r.root)
            .map(PathBuf::from)
    }

    /// Remember which project a workspace key belongs to — the key is a
    /// hash, so without this a per-workspace breakdown can only show ids.
    /// Write-once: the value never changes for a given key.
    pub fn record_root(&self, root: &Path) {
        let Ok(Some(mut row)) = self.workspace_row() else {
            return;
        };
        if row.root.is_some() {
            return;
        }
        row.root = Some(root.to_string_lossy().into_owned());
        let _ = self.put_workspace_row(&row);
    }

    /// Last resort for workspaces imported before roots were recorded: the
    /// session's system prompt names the workspace (`Workspace root: <path>`),
    /// and it sits in the first message of a history snapshot. Only a handful
    /// of sessions are checked — this is a bounded read.
    pub fn recover_root_from_history(&self) -> Option<PathBuf> {
        let rtx = self.db.begin_read().ok()?;
        let t = rtx.open_table(HISTORY).ok()?;
        let prefix = format!("{}/", self.ws);
        let iter = t.range::<&str>(prefix.as_str()..).ok()?;
        for e in iter.flatten().take(4) {
            let Ok(hist) = serde_json::from_slice::<Vec<ChatMessage>>(e.1.value()) else {
                continue;
            };
            for msg in hist.iter().take(2) {
                let Some(c) = msg.content.as_deref() else {
                    continue;
                };
                if let Some(rest) = c.split("Workspace root: ").nth(1) {
                    let path = rest.split(&['\\', '"', '\n'][..]).next()?.trim();
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                }
            }
        }
        None
    }

    /// Read a per-session scratch blob (`todos.json`, …) — replaces the
    /// sibling `<id>.<name>` files.
    pub fn read_state(&self, id: i64, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        let key = format!("{}/{}/{}", self.ws, id, name);
        let rtx = self.db.begin_read().map_err(io_err)?;
        let t = rtx.open_table(STATE).map_err(io_err)?;
        Ok(t.get(key.as_str())
            .map_err(io_err)?
            .map(|g| g.value().to_vec()))
    }

    /// Write a per-session scratch blob — a single atomic commit.
    pub fn write_state(&self, id: i64, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        let key = format!("{}/{}/{}", self.ws, id, name);
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(STATE).map_err(io_err)?;
            t.insert(key.as_str(), bytes).map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)
    }

    /// All metas for this workspace (unsorted).
    fn metas(&self) -> Vec<SessionMeta> {
        let Ok(rtx) = self.db.begin_read() else {
            return Vec::new();
        };
        let Ok(t) = rtx.open_table(METAS) else {
            return Vec::new();
        };
        let prefix = format!("{}/", self.ws);
        let Ok(iter) = t.range::<&str>(prefix.as_str()..) else {
            return Vec::new();
        };
        iter.flatten()
            .take_while(|(k, _)| k.value().starts_with(&prefix))
            .filter_map(|(_, v)| serde_json::from_slice::<SessionMeta>(v.value()).ok())
            .collect()
    }

    /// List sessions — pinned first, then newest activity within each tier.
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut sessions = self.metas();
        sessions.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        sessions
    }

    /// Toggle a session's pinned flag; returns whether the session existed.
    pub fn set_pinned(&self, id: i64, pinned: bool) -> std::io::Result<bool> {
        let key = format!("{}/{}", self.ws, id);
        let wtx = self.db.begin_write().map_err(io_err)?;
        let found = {
            let mut t = wtx.open_table(METAS).map_err(io_err)?;
            let Some(g) = t.get(key.as_str()).map_err(io_err)? else {
                return Ok(false);
            };
            let mut meta: SessionMeta = serde_json::from_slice(g.value()).map_err(io_err)?;
            drop(g);
            meta.pinned = pinned;
            let json = serde_json::to_vec(&meta).map_err(io_err)?;
            t.insert(key.as_str(), json.as_slice()).map_err(io_err)?;
            true
        };
        wtx.commit().map_err(io_err)?;
        Ok(found)
    }

    /// The session the user last had open — `None` on a fresh store.
    pub fn last_active(&self) -> Option<i64> {
        self.workspace_row()
            .ok()
            .flatten()
            .and_then(|r| r.last_active)
    }

    /// Record which session is open — restored on next launch.
    pub fn set_last_active(&self, id: i64) -> std::io::Result<()> {
        let mut row = self.workspace_row()?.unwrap_or_default();
        if row.last_active == Some(id) {
            return Ok(());
        }
        row.last_active = Some(id);
        self.put_workspace_row(&row)
    }

    /// Load the latest history snapshot for a session.
    pub fn load_history(&self, id: i64) -> Option<Vec<ChatMessage>> {
        let rtx = self.db.begin_read().ok()?;
        let t = rtx.open_table(HISTORY).ok()?;
        let key = format!("{}/{}", self.ws, id);
        let g = t.get(key.as_str()).ok()??;
        serde_json::from_slice(g.value()).ok()
    }

    /// Store the latest history snapshot for a session (overwrites). Called
    /// at turn boundaries — the commit is atomic, so a crash mid-write leaves
    /// the PREVIOUS snapshot intact instead of a torn record.
    pub fn snapshot(&self, id: i64, history: &[ChatMessage]) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(history)?;
        let key = format!("{}/{}", self.ws, id);
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(HISTORY).map_err(io_err)?;
            t.insert(key.as_str(), bytes.as_slice()).map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)
    }

    /// Update sidebar metadata for a session (insert or replace by id).
    pub fn upsert_meta(&self, meta: SessionMeta) -> std::io::Result<()> {
        let key = format!("{}/{}", self.ws, meta.id);
        let json = serde_json::to_vec(&meta)?;
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(METAS).map_err(io_err)?;
            t.insert(key.as_str(), json.as_slice()).map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)
    }

    /// Allocate a fresh session id (max existing + 1, starting at 1).
    pub fn next_id(&self) -> i64 {
        self.metas().iter().map(|s| s.id).max().unwrap_or(0) + 1
    }

    /// Remove a session entirely — meta, history, and its `<id>/<name>`
    /// scratch rows. `last_active` pointing at it is cleared, matching the
    /// old index surgery.
    pub fn remove(&self, id: i64) -> std::io::Result<()> {
        let meta_key = format!("{}/{}", self.ws, id);
        let state_prefix = format!("{}/{}/", self.ws, id);
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(METAS).map_err(io_err)?;
            t.remove(meta_key.as_str()).map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(HISTORY).map_err(io_err)?;
            t.remove(meta_key.as_str()).map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(STATE).map_err(io_err)?;
            t.retain(|k, _| !k.starts_with(&state_prefix))
                .map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(WORKSPACES).map_err(io_err)?;
            let row: Option<WorkspaceRow> = t
                .get(self.ws.as_str())
                .map_err(io_err)?
                .and_then(|g| serde_json::from_slice(g.value()).ok());
            if let Some(mut row) = row {
                if row.last_active == Some(id) {
                    row.last_active = None;
                    let json = serde_json::to_vec(&row).map_err(io_err)?;
                    t.insert(self.ws.as_str(), json.as_slice())
                        .map_err(io_err)?;
                }
            }
        }
        wtx.commit().map_err(io_err)
    }

    /// Drop EVERY row under this workspace's namespace — sessions, history,
    /// scratch state, prefs, and the workspace row itself. Used by tests to
    /// undo `open()` on a throwaway root.
    pub fn delete_workspace(&self) -> std::io::Result<()> {
        let prefix = format!("{}/", self.ws);
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(METAS).map_err(io_err)?;
            t.retain(|k, _| !k.starts_with(&prefix)).map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(HISTORY).map_err(io_err)?;
            t.retain(|k, _| !k.starts_with(&prefix)).map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(STATE).map_err(io_err)?;
            t.retain(|k, _| !k.starts_with(&prefix)).map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(PREFS).map_err(io_err)?;
            t.remove(self.ws.as_str()).map_err(io_err)?;
        }
        {
            let mut t = wtx.open_table(WORKSPACES).map_err(io_err)?;
            t.remove(self.ws.as_str()).map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)
    }

    /// Load persisted prefs (empty if absent/corrupt).
    pub fn load_prefs(&self) -> WorkspacePrefs {
        let Ok(rtx) = self.db.begin_read() else {
            return WorkspacePrefs::default();
        };
        let Ok(t) = rtx.open_table(PREFS) else {
            return WorkspacePrefs::default();
        };
        t.get(self.ws.as_str())
            .ok()
            .flatten()
            .and_then(|g| serde_json::from_slice(g.value()).ok())
            .unwrap_or_default()
    }

    /// Persist prefs (overwrite).
    pub fn save_prefs(&self, prefs: &WorkspacePrefs) -> std::io::Result<()> {
        let json = serde_json::to_vec(prefs)?;
        let wtx = self.db.begin_write().map_err(io_err)?;
        {
            let mut t = wtx.open_table(PREFS).map_err(io_err)?;
            t.insert(self.ws.as_str(), json.as_slice())
                .map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)
    }
}

/// Stable per-workspace key — xxh3 of the canonical root.
pub fn workspace_key(root: &Path) -> String {
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

/// Open (creating) the sessions database at `path`, running the legacy
/// file import once. `path.parent()` is the data dir — the legacy
/// `sessions/` tree and the global `*.json` files live next to the db.
fn open_db(path: &Path) -> std::io::Result<Database> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut db = Database::create(path).map_err(io_err)?;
    // Materialize every table eagerly — a fresh db has none, and a read-tx
    // `open_table` on a missing table errors instead of reading empty.
    {
        let wtx = db.begin_write().map_err(io_err)?;
        for def in [METAS, HISTORY, STATE, PREFS, WORKSPACES, GLOBALS] {
            wtx.open_table(def).map_err(io_err)?;
        }
        wtx.commit().map_err(io_err)?;
    }
    if migrate_legacy(&db, path) {
        // The one-shot import leaves the file at ~3× payload (CoW pages);
        // compact it while nothing else holds the handle.
        let _ = db.compact();
    }
    Ok(db)
}

fn global_db() -> Option<Arc<Database>> {
    static DB: OnceLock<Option<Arc<Database>>> = OnceLock::new();
    DB.get_or_init(|| {
        let path = app_data_dir()?.join(DB_FILE);
        open_db(&path).ok().map(Arc::new)
    })
    .clone()
}

// ---------------------------------------------------------------------------
// Legacy import — sessions/<ws>/{index.json,<id>.jsonl,prefs.json,workspace.json,
// <id>.*} plus the three app-level json files, all in ONE write tx so a crash
// leaves nothing half-migrated. Files are renamed aside only after the commit
// succeeds; a failure simply retries on the next open.
// ---------------------------------------------------------------------------

/// Returns whether anything was migrated (the caller compacts afterwards).
fn migrate_legacy(db: &Database, db_path: &Path) -> bool {
    let Some(data_dir) = db_path.parent() else {
        return false;
    };
    let sessions_dir = data_dir.join("sessions");
    const GLOBAL_FILES: [(&str, &str); 3] = [
        ("recent_workspaces.json", "recent_workspaces"),
        ("default_preferences.json", "default_preferences"),
        ("appearance.json", "appearance"),
    ];
    let has_sessions = sessions_dir.is_dir();
    let has_globals = GLOBAL_FILES.iter().any(|(f, _)| data_dir.join(f).is_file());
    if !has_sessions && !has_globals {
        return false;
    }

    let Ok(wtx) = db.begin_write() else {
        return false;
    };
    let imported = (|| -> std::io::Result<()> {
        if has_sessions {
            let mut metas = wtx.open_table(METAS).map_err(io_err)?;
            let mut history = wtx.open_table(HISTORY).map_err(io_err)?;
            let mut state = wtx.open_table(STATE).map_err(io_err)?;
            let mut prefs = wtx.open_table(PREFS).map_err(io_err)?;
            let mut workspaces = wtx.open_table(WORKSPACES).map_err(io_err)?;
            for e in std::fs::read_dir(&sessions_dir)?.flatten() {
                let dir = e.path();
                if !dir.is_dir() {
                    continue;
                }
                let ws = e.file_name().to_string_lossy().into_owned();
                let mut row = WorkspaceRow::default();
                if let Ok(text) = std::fs::read_to_string(dir.join("workspace.json")) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        row.root = v
                            .get("root")
                            .and_then(|r| r.as_str())
                            .map(|s| s.to_string());
                    }
                }
                if let Ok(text) = std::fs::read_to_string(dir.join("index.json")) {
                    if let Ok(idx) = serde_json::from_str::<SessionIndex>(&text) {
                        row.last_active = idx.last_active;
                        for meta in idx.sessions {
                            let key = format!("{}/{}", ws, meta.id);
                            if let Ok(j) = serde_json::to_vec(&meta) {
                                let _ = metas.insert(key.as_str(), j.as_slice());
                            }
                        }
                    }
                }
                if let Ok(bytes) = std::fs::read(dir.join("prefs.json")) {
                    let _ = prefs.insert(ws.as_str(), bytes.as_slice());
                }
                let _ = workspaces.insert(ws.as_str(), {
                    serde_json::to_vec(&row).unwrap_or_default().as_slice()
                });
                for e in std::fs::read_dir(&dir)?.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if let Some(id) = name
                        .strip_suffix(".jsonl")
                        .and_then(|s| s.parse::<i64>().ok())
                    {
                        if let Some(bytes) = last_intact_jsonl_line(&e.path()) {
                            let key = format!("{}/{}", ws, id);
                            let _ = history.insert(key.as_str(), bytes.as_slice());
                        }
                    } else if let Some(rest) = name.split_once('.').map(|(_, r)| r) {
                        // `<id>.<name>` siblings (todos, scratch) — the first
                        // dot splits id from name; index/prefs/workspace
                        // aren't id-prefixed and were handled above.
                        if let Ok(id) = name.split('.').next().unwrap_or("").parse::<i64>() {
                            if let Ok(bytes) = std::fs::read(e.path()) {
                                let key = format!("{}/{}/{}", ws, id, rest);
                                let _ = state.insert(key.as_str(), bytes.as_slice());
                            }
                        }
                    }
                }
            }
        }
        {
            let mut globals = wtx.open_table(GLOBALS).map_err(io_err)?;
            for (file, key) in GLOBAL_FILES {
                if let Ok(bytes) = std::fs::read(data_dir.join(file)) {
                    let _ = globals.insert(key, bytes.as_slice());
                }
            }
        }
        Ok(())
    })();
    if imported.is_err() || wtx.commit().is_err() {
        // Nothing committed — the files stay, the next open retries.
        return false;
    }

    // Commit landed: move the sources aside (recoverable, and it marks the
    // import done — a later `sessions/` dir would simply re-import).
    let _ = std::fs::rename(&sessions_dir, data_dir.join("sessions.legacy"));
    for (file, _) in GLOBAL_FILES {
        let _ = std::fs::rename(data_dir.join(file), data_dir.join(format!("{file}.bak")));
    }
    true
}

/// The newest complete line in an append-only `.jsonl` — the migration
/// equivalent of the old `load_history` tail scan. A torn trailing line is
/// skipped, so the last INTACT snapshot is what gets imported.
fn last_intact_jsonl_line(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    for line in bytes.split(|b| *b == b'\n').rev() {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if serde_json::from_slice::<Vec<ChatMessage>>(line).is_ok() {
            return Some(line.to_vec());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Workspace preferences — per-workspace UI prefs (model / thinking / permission)
// persisted in the `prefs` table. Loaded at `SessionManager` boot and fed into
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
    /// Per-workspace agent mode (`build` | `plan` | `goal`).
    #[serde(default)]
    pub agent_mode: Option<String>,
}

/// Global user default preferences for fresh/new sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultPreferences {
    #[serde(default = "default_pref_permission_mode")]
    pub permission_mode: String,
    #[serde(default = "default_pref_thinking_level")]
    pub thinking_level: Option<String>,
    #[serde(default = "default_pref_agent_mode")]
    pub agent_mode: String,
    /// Fraction of the context window that triggers compaction (0.70/0.80/0.90).
    #[serde(default = "default_pref_compact_at")]
    pub compact_at: f32,
    /// Sandbox network override: "auto" (audit decides) | "allow" | "deny".
    #[serde(default)]
    pub sandbox_network: Option<String>,
    /// Per-command sandbox memory cap override (MB).
    #[serde(default)]
    pub sandbox_max_memory_mb: Option<u64>,
    /// Per-command sandbox process-count cap override.
    #[serde(default)]
    pub sandbox_max_processes: Option<u32>,
    /// Memory subsystem master switch — `false` opens no `MemoryStore`: no
    /// `<memory>` block in the prompt and no episode/fact writes.
    #[serde(default = "default_pref_bool_true")]
    pub memory_enabled: bool,
    /// Model distillation — `false` still writes episodes/facts via the
    /// deterministic path but skips the per-turn summarizer model call.
    #[serde(default = "default_pref_bool_true")]
    pub memory_distill: bool,
}

fn default_pref_bool_true() -> bool {
    true
}

fn default_pref_compact_at() -> f32 {
    0.80
}

fn default_pref_permission_mode() -> String {
    "auto".into()
}

fn default_pref_agent_mode() -> String {
    "build".into()
}

fn default_pref_thinking_level() -> Option<String> {
    Some("medium".into())
}

impl Default for DefaultPreferences {
    fn default() -> Self {
        Self {
            permission_mode: default_pref_permission_mode(),
            thinking_level: default_pref_thinking_level(),
            agent_mode: default_pref_agent_mode(),
            compact_at: default_pref_compact_at(),
            sandbox_network: None,
            sandbox_max_memory_mb: None,
            sandbox_max_processes: None,
            memory_enabled: true,
            memory_distill: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Appearance settings — the settings window's theme/accent/font choices.
// The frontend reads it at boot and applies `dark` class + CSS vars.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// "system" | "light" | "dark" — `system` follows the OS via
    /// `prefers-color-scheme`, the others pin the class directly.
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// Active light/dark theme id (reserved — theme packs land later).
    #[serde(default)]
    pub theme_id: Option<String>,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_bg")]
    pub background: String,
    #[serde(default = "default_fg")]
    pub foreground: String,
    /// Dark-mode overrides — when set, `applyAppearance` uses these while
    /// the effective mode is dark; absent → the `.dark` palette defaults.
    #[serde(default)]
    pub dark_accent: Option<String>,
    #[serde(default)]
    pub dark_background: Option<String>,
    #[serde(default)]
    pub dark_foreground: Option<String>,
    #[serde(default = "default_ui_font")]
    pub ui_font: String,
    #[serde(default = "default_code_font")]
    pub code_font: String,
    /// 0–100 contrast slider value (UI hint, maps to a subtle text boost).
    #[serde(default = "default_contrast")]
    pub contrast: u32,
    /// Cost display currency — "usd" | "cny". A display hint only: the
    /// title-bar chip renders the same number with `$` or `¥`; no FX
    /// conversion happens anywhere.
    #[serde(default = "default_currency")]
    pub currency: String,
}

fn default_theme_mode() -> String {
    "system".into()
}
fn default_accent() -> String {
    "#339CFF".into()
}
fn default_bg() -> String {
    "#FFFFFF".into()
}
fn default_fg() -> String {
    "#1A1C1F".into()
}
fn default_ui_font() -> String {
    "-apple-system, BlinkMacSystemFont, \"Segoe UI\"".into()
}
fn default_code_font() -> String {
    "ui-monospace, \"SFMono-Regular\", monospace".into()
}
fn default_contrast() -> u32 {
    45
}
fn default_currency() -> String {
    "usd".into()
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme_mode: default_theme_mode(),
            theme_id: None,
            accent: default_accent(),
            background: default_bg(),
            foreground: default_fg(),
            dark_accent: None,
            dark_background: None,
            dark_foreground: None,
            ui_font: default_ui_font(),
            code_font: default_code_font(),
            contrast: default_contrast(),
            currency: default_currency(),
        }
    }
}

// ---------------------------------------------------------------------------
// Global rows — app-level singletons in the `globals` table.
// ---------------------------------------------------------------------------

fn get_global<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
    let db = global_db()?;
    let rtx = db.begin_read().ok()?;
    let t = rtx.open_table(GLOBALS).ok()?;
    let g = t.get(key).ok()??;
    serde_json::from_slice(g.value()).ok()
}

fn put_global(key: &str, value: &impl Serialize) -> std::io::Result<()> {
    let Some(db) = global_db() else {
        return Ok(());
    };
    let json = serde_json::to_vec(value)?;
    let wtx = db.begin_write().map_err(io_err)?;
    {
        let mut t = wtx.open_table(GLOBALS).map_err(io_err)?;
        t.insert(key, json.as_slice()).map_err(io_err)?;
    }
    wtx.commit().map_err(io_err)
}

/// Recently opened workspace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentWorkspace {
    pub path: PathBuf,
    pub name: String,
    pub last_opened: u64,
}

pub fn load_recent_workspaces() -> Vec<RecentWorkspace> {
    let mut list: Vec<RecentWorkspace> = get_global("recent_workspaces").unwrap_or_default();
    list.retain(|w| w.path.is_dir());
    list.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    list
}

pub fn record_recent_workspace(workspace_root: &Path) {
    let canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
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
    list.insert(
        0,
        RecentWorkspace {
            path: canon,
            name,
            last_opened: now,
        },
    );
    list.truncate(15);

    let _ = put_global("recent_workspaces", &list);
}

/// Drop a workspace from recents; session rows are left in the db.
pub fn remove_recent_workspace(workspace_root: &Path) {
    let canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut list = load_recent_workspaces();
    list.retain(|w| w.path != canon);
    let _ = put_global("recent_workspaces", &list);
}

pub fn load_default_preferences() -> DefaultPreferences {
    get_global("default_preferences").unwrap_or_default()
}

/// Load the global default preferences only when a row actually exists —
/// `None` when absent or corrupt. Used as a *fallback* layer below
/// per-workspace prefs so an untouched install keeps the built-in defaults.
pub fn try_load_default_preferences() -> Option<DefaultPreferences> {
    get_global("default_preferences")
}

pub fn save_default_preferences(prefs: &DefaultPreferences) -> std::io::Result<()> {
    put_global("default_preferences", prefs)
}

pub fn load_appearance_settings() -> AppearanceSettings {
    get_global("appearance").unwrap_or_default()
}

pub fn save_appearance_settings(s: &AppearanceSettings) -> std::io::Result<()> {
    put_global("appearance", s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_llm::types::ChatMessage;

    /// An isolated store: temp db file + the tempdir as the workspace root.
    fn test_store(dir: &tempfile::TempDir) -> SessionStore {
        let db = Arc::new(open_db(&dir.path().join("t.db")).unwrap());
        SessionStore::open_with(dir.path(), db).unwrap()
    }

    fn meta(id: i64, updated_at: u64) -> SessionMeta {
        SessionMeta {
            id,
            title: format!("s{id}"),
            preview: String::new(),
            updated_at,
            pinned: false,
            usage: None,
            model: None,
            provider: None,
            permission_mode: None,
            agent_mode: None,
            thinking_level: None,
            queued_prompts: Vec::new(),
        }
    }

    #[test]
    fn snapshot_overwrites_so_only_the_latest_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        store.snapshot(7, &[ChatMessage::user("first")]).unwrap();
        store
            .snapshot(
                7,
                &[ChatMessage::user("first"), ChatMessage::assistant("one")],
            )
            .unwrap();
        let hist = store.load_history(7).expect("history");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[1].content.as_deref(), Some("one"));

        // Missing id → None.
        assert!(store.load_history(9).is_none());
    }

    #[test]
    fn metas_list_sorts_pinned_then_recency_and_next_id_grows() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        store.upsert_meta(meta(1, 100)).unwrap();
        store.upsert_meta(meta(2, 50)).unwrap();
        store.upsert_meta(meta(3, 10)).unwrap();
        assert_eq!(store.next_id(), 4);

        store.set_pinned(3, true).unwrap();
        let ids: Vec<i64> = store.list().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![3, 1, 2]);

        // upsert replaces by id.
        store.upsert_meta(meta(2, 500)).unwrap();
        let ids: Vec<i64> = store.list().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![3, 2, 1]);
    }

    #[test]
    fn remove_drops_meta_history_state_and_last_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        store.upsert_meta(meta(7, 1)).unwrap();
        store.snapshot(7, &[ChatMessage::user("hi")]).unwrap();
        store.write_state(7, "todos.json", b"{}").unwrap();
        store.set_last_active(7).unwrap();

        store.remove(7).unwrap();
        assert!(store.list().is_empty());
        assert!(store.load_history(7).is_none());
        assert!(store.read_state(7, "todos.json").unwrap().is_none());
        assert_eq!(store.last_active(), None);
    }

    #[test]
    fn prefs_and_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        let prefs = WorkspacePrefs {
            provider: Some("devin".into()),
            model: Some("devin/swe-2".into()),
            thinking_level: Some("high".into()),
            permission_mode: Some("default".into()),
            agent_mode: Some("plan".into()),
        };
        store.save_prefs(&prefs).unwrap();
        let loaded = store.load_prefs();
        assert_eq!(loaded.thinking_level.as_deref(), Some("high"));
        assert_eq!(loaded.model.as_deref(), Some("devin/swe-2"));

        store
            .write_state(0, "todos.json", b"{\"next_id\":2}")
            .unwrap();
        assert_eq!(
            store.read_state(0, "todos.json").unwrap().as_deref(),
            Some(b"{\"next_id\":2}".as_slice())
        );
    }

    #[test]
    fn workspace_row_carries_root_and_last_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        assert_eq!(store.recorded_root(), Some(dir.path().to_path_buf()));
        // record_root is write-once.
        store.record_root(Path::new("/elsewhere"));
        assert_eq!(store.recorded_root(), Some(dir.path().to_path_buf()));

        store.set_last_active(4).unwrap();
        assert_eq!(store.last_active(), Some(4));
    }

    #[test]
    fn migrate_legacy_imports_the_json_tree() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let ws_dir = data.join("sessions").join("aa11bb22cc33dd44");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("workspace.json"), br#"{"root":"/proj/demo"}"#).unwrap();
        std::fs::write(
            ws_dir.join("index.json"),
            r#"{"last_active":7,"sessions":[{"id":7,"title":"t","preview":"","updated_at":9}]}"#,
        )
        .unwrap();
        std::fs::write(ws_dir.join("prefs.json"), br#"{"provider":"devin"}"#).unwrap();
        std::fs::write(
            ws_dir.join("7.jsonl"),
            concat!(
                r#"[{"role":"user","content":"old"}]"#,
                "\n",
                r#"[{"role":"user","content":"old"},{"role":"assistant","content":"new"}]"#,
                "\n",
                // torn trailing record — must be skipped
                r#"[{"role":"user","content":"brot"#
            ),
        )
        .unwrap();
        std::fs::write(ws_dir.join("7.todos.json"), b"{}").unwrap();
        std::fs::write(
            data.join("recent_workspaces.json"),
            r#"[{"path":"/proj/demo","name":"demo","last_opened":3}]"#,
        )
        .unwrap();
        std::fs::write(data.join("appearance.json"), br##"{"accent":"#000"}"##).unwrap();

        // `open_db` triggers the migration; the imported rows are keyed by
        // the legacy DIRECTORY name, so bind a store straight to that key.
        let store = SessionStore {
            ws: "aa11bb22cc33dd44".into(),
            db: Arc::new(open_db(&data.join(DB_FILE)).unwrap()),
        };
        let hist = store.load_history(7).expect("imported history");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[1].content.as_deref(), Some("new"));
        let m = &store.list()[0];
        assert_eq!(m.id, 7);
        assert_eq!(store.last_active(), Some(7));
        assert_eq!(store.load_prefs().provider.as_deref(), Some("devin"));
        assert_eq!(store.recorded_root(), Some(PathBuf::from("/proj/demo")));
        assert!(store.read_state(7, "todos.json").unwrap().is_some());

        // Sources were moved aside after the commit.
        assert!(!data.join("sessions").exists());
        assert!(data.join("sessions.legacy").is_dir());
        assert!(!data.join("recent_workspaces.json").exists());
        assert!(data.join("recent_workspaces.json.bak").is_file());
        assert!(data.join("appearance.json.bak").is_file());
    }
}
