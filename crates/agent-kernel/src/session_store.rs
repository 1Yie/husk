//! `session_store` — per-workspace session persistence.
//!
//! `~/.local/share/husk/sessions/<ws_hash>/`:
//!   - `index.json`  — session metadata (title/preview/updated_at)
//!   - `<id>.jsonl`  — one `Vec<ChatMessage>` per line, appended per turn;
//!                     the last line wins, so a crash costs at most the
//!                     current turn. Rotated to that last line once the file
//!                     passes [`MAX_HISTORY_BYTES`] (earlier lines are never
//!                     read).
//!
//! The owning `SessionActor` writes on turn boundaries only (single writer,
//! never mid-turn), so a running session's file holds its last completed state.

use std::path::{Path, PathBuf};

use agent_llm::types::ChatMessage;
use serde::{Deserialize, Serialize};

/// Rotate the history file once it passes this size: only the LAST snapshot
/// line is ever read, so the earlier ones are pure overhead. Without rotation
/// the file grows with `turns × history size` (quadratic) and never shrinks —
/// a long session reaches hundreds of MB while `load_history` still reads one
/// line's worth.
const MAX_HISTORY_BYTES: u64 = 8 * 1024 * 1024;

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

/// The on-disk session index for one workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    /// The session the user last had open — restored on next launch
    /// instead of defaulting to the most recently updated one.
    #[serde(default)]
    pub last_active: Option<i64>,
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
        let store = Self { dir: base };
        // Remember which project this dir belongs to, while the root is known.
        store.record_root(workspace_root);
        Ok(store)
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
        if !base.is_dir() {
            return None;
        }
        let store = Self { dir: base };
        store.record_root(workspace_root);
        Some(store)
    }

    /// Every workspace store on disk (`sessions/*`). The 统计 pane aggregates
    /// across ALL of them: recents only remembers the projects the user opened
    /// through this install, so anything older — or any project whose recent
    /// entry was pruned — was invisible in the totals.
    pub fn all_workspaces() -> Vec<Self> {
        let Some(base) = app_data_dir().map(|d| d.join("sessions")) else {
            return Vec::new();
        };
        let Ok(rd) = std::fs::read_dir(&base) else {
            return Vec::new();
        };
        rd.flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| Self { dir: e.path() })
            .collect()
    }

    /// This store's directory name (the workspace key hash).
    pub fn dir_name(&self) -> Option<String> {
        self.dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// The workspace root this store belongs to, if it was ever recorded.
    pub fn recorded_root(&self) -> Option<PathBuf> {
        let text = std::fs::read_to_string(self.dir.join("workspace.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v.get("root").and_then(|r| r.as_str()).map(PathBuf::from)
    }

    /// Remember which project a session dir belongs to — the dir name is a
    /// hash, so without this a per-workspace breakdown can only show ids.
    /// Write-once: the value never changes for a given key.
    pub fn record_root(&self, root: &Path) {
        if self.recorded_root().is_some() {
            return;
        }
        let body = serde_json::json!({ "root": root.to_string_lossy() });
        let _ = std::fs::write(
            self.dir.join("workspace.json"),
            serde_json::to_string(&body).unwrap_or_default(),
        );
    }

    /// Last resort for dirs written before roots were recorded: the session's
    /// system prompt names the workspace (`Workspace root: <path>`), and it sits
    /// in the first line of the smallest history file. Bounded read — a few KB.
    pub fn recover_root_from_history(&self) -> Option<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));
        for f in files.iter().take(4) {
            let Ok(mut fh) = std::fs::File::open(f) else {
                continue;
            };
            let mut head = vec![0u8; 32 * 1024];
            use std::io::Read as _;
            let n = fh.read(&mut head).unwrap_or(0);
            head.truncate(n);
            let text = String::from_utf8_lossy(&head);
            if let Some(rest) = text.split("Workspace root: ").nth(1) {
                let path = rest.split(&['\\', '"', '\n'][..]).next()?.trim();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
        None
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
        idx.sessions.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
        });
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

    /// The session the user last had open — `None` on a fresh/old index.
    pub fn last_active(&self) -> Option<i64> {
        self.read_index().last_active
    }

    /// Record which session is open — restored on next launch.
    pub fn set_last_active(&self, id: i64) -> std::io::Result<()> {
        let mut idx = self.read_index();
        if idx.last_active == Some(id) {
            return Ok(());
        }
        idx.last_active = Some(id);
        let json = serde_json::to_string_pretty(&idx)?;
        std::fs::write(self.index_path(), json)
    }

    /// Load the latest history snapshot for a session.
    ///
    /// The file is append-only full snapshots, so it grows roughly with
    /// `turns × history size` while only the tail line is ever read. Read
    /// backward in 64 KiB blocks until the last line's leading newline is
    /// found instead of pulling the whole file into memory — a 100 MB
    /// history file then costs one snapshot's worth of I/O, not the whole
    /// file, on every `history_page` fetch.
    pub fn load_history(&self, id: i64) -> Option<Vec<ChatMessage>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(self.history_path(id)).ok()?;
        let mut pos = f.metadata().ok()?.len();
        const BLOCK: u64 = 64 * 1024;
        // `tail` holds the file's trailing bytes read so far and grows
        // toward the head until the final non-blank line is complete.
        let mut tail: Vec<u8> = Vec::new();
        loop {
            let step = pos.min(BLOCK);
            pos -= step;
            f.seek(SeekFrom::Start(pos)).ok()?;
            let mut buf = vec![0u8; step as usize];
            f.read_exact(&mut buf).ok()?;
            buf.extend_from_slice(&tail);
            tail = buf;
            if let Some(start) = last_line_start(&tail) {
                return serde_json::from_slice(&tail[start..]).ok();
            }
            if pos == 0 {
                // The whole file is one unterminated line (or all blank).
                let line = trim_ascii_end(&tail);
                if line.is_empty() {
                    return None;
                }
                return serde_json::from_slice(line).ok();
            }
        }
    }

    /// Append a full history snapshot (one line). Called at turn boundaries.
    ///
    /// Past [`MAX_HISTORY_BYTES`] the file is rewritten to just this snapshot —
    /// atomically, via a temp file + rename, so a crash mid-rotation leaves the
    /// original intact (the reader only ever wants the last line anyway).
    pub fn snapshot(&self, id: i64, history: &[ChatMessage]) -> std::io::Result<()> {
        use std::io::Write;
        let line = serde_json::to_string(history)?;
        let path = self.history_path(id);
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            f.write_all(line.as_bytes())?;
            f.write_all(b"\n")?;
        }
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_HISTORY_BYTES {
            let tmp = path.with_extension("jsonl.tmp");
            {
                let mut t = std::fs::File::create(&tmp)?;
                t.write_all(line.as_bytes())?;
                t.write_all(b"\n")?;
                t.sync_all()?;
            }
            std::fs::rename(&tmp, &path)?;
        }
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
        if idx.last_active == Some(id) {
            idx.last_active = None;
        }
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

/// Byte offset where `buf`'s final non-blank line begins — i.e. just past
/// the last `'\n'` that precedes real content. `None` when `buf` holds no
/// newline before its last content byte (the line may extend beyond what
/// has been read so far — the caller keeps reading toward the head).
fn last_line_start(buf: &[u8]) -> Option<usize> {
    let end = trim_ascii_end(buf).len();
    if end == 0 {
        return None;
    }
    buf[..end]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|nl| nl + 1)
}

/// `buf` minus trailing ASCII whitespace/newlines — the final snapshot
/// line may be followed by nothing but `'\n'` padding.
fn trim_ascii_end(buf: &[u8]) -> &[u8] {
    let mut end = buf.len();
    while end > 0 && buf[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &buf[..end]
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
    let Some(path) = recent_workspaces_path() else {
        return Vec::new();
    };
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
        }
    }
}

pub fn default_preferences_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("default_preferences.json"))
}

pub fn load_default_preferences() -> DefaultPreferences {
    let Some(path) = default_preferences_path() else {
        return DefaultPreferences::default();
    };
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
    let Some(path) = default_preferences_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(prefs)?;
    std::fs::write(path, json)
}

// ---------------------------------------------------------------------------
// Appearance settings — the settings window's theme/accent/font choices,
// persisted at the app-data root next to `default_preferences.json`.
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

pub fn appearance_settings_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("appearance.json"))
}

pub fn load_appearance_settings() -> AppearanceSettings {
    let Some(path) = appearance_settings_path() else {
        return AppearanceSettings::default();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_appearance_settings(s: &AppearanceSettings) -> std::io::Result<()> {
    let Some(path) = appearance_settings_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(s)?;
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
    let Some(path) = recent_workspaces_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
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

    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(path, json);
    }
}

/// Drop a workspace from recents; session files are left on disk.
pub fn remove_recent_workspace(workspace_root: &Path) {
    let Some(path) = recent_workspaces_path() else {
        return;
    };
    let canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut list = load_recent_workspaces();
    list.retain(|w| w.path.canonicalize().unwrap_or_else(|_| w.path.clone()) != canon);
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_llm::types::ChatMessage;

    /// The reader must pick the LAST non-blank line even when earlier
    /// snapshots push it more than one 64 KiB read block from the EOF —
    /// this is what keeps `history_page` O(snapshot) instead of O(file).
    #[test]
    fn load_history_finds_last_line_across_block_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore {
            dir: dir.path().to_path_buf(),
        };

        store.snapshot(7, &[ChatMessage::user("first")]).unwrap();
        // A mid-file snapshot inflated past the 64 KiB block size, so the
        // final small line lands >1 block from the start and the backward
        // reader must assemble it across reads.
        let big = ChatMessage::user("x".repeat(200 * 1024));
        store
            .snapshot(7, &[big, ChatMessage::assistant("mid")])
            .unwrap();
        store
            .snapshot(
                7,
                &[ChatMessage::user("final"), ChatMessage::assistant("answer")],
            )
            .unwrap();
        // Trailing blank lines must not hide the real last line.
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("7.jsonl"))
            .map(|mut f| {
                use std::io::Write;
                let _ = f.write_all(b"\n\n");
            })
            .unwrap();

        let hist = store.load_history(7).expect("history");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content.as_deref(), Some("final"));
        assert_eq!(hist[1].content.as_deref(), Some("answer"));

        // Missing file → None; empty file → None.
        assert!(store.load_history(9).is_none());
        std::fs::write(dir.path().join("8.jsonl"), b"\n\n").unwrap();
        assert!(store.load_history(8).is_none());
    }

    /// Snapshots are full copies, so the file grew with `turns × history` and
    /// never shrank. Past the cap only the last line survives, and the reader
    /// still returns that newest snapshot.
    #[test]
    fn snapshot_rotation_keeps_only_the_newest_line() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore {
            dir: dir.path().to_path_buf(),
        };
        let path = dir.path().join("7.jsonl");
        let lines = || {
            std::fs::read_to_string(&path)
                .map(|t| t.lines().count())
                .unwrap_or(0)
        };

        // A single oversized snapshot trips the cap immediately.
        let big = ChatMessage::user("x".repeat(MAX_HISTORY_BYTES as usize + 4096));
        store.snapshot(7, std::slice::from_ref(&big)).unwrap();
        assert_eq!(lines(), 1);
        assert!(std::fs::read_to_string(&path).unwrap().contains("xxx"));

        // The next snapshot exceeds the cap again → the huge line is dropped.
        store.snapshot(7, &[ChatMessage::user("after")]).unwrap();
        assert_eq!(lines(), 1, "rotation must keep exactly one line");
        assert!(
            !std::fs::read_to_string(&path).unwrap().contains("xxx"),
            "the pre-rotation line must be gone"
        );
        let hist = store.load_history(7).expect("history after rotation");
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].content.as_deref(), Some("after"));
    }
}
