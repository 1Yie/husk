//! File-based session repository: independent JSONL journals and atomic indexes.
//! Each window opens files on demand; only individual writes use short locks.
//! Old redb data is imported from a verified snapshot without locking the live
//! database or stopping another frontend. Original sources remain unchanged.

mod files;
mod migration;
pub use files::SessionDatabase;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use agent_llm::types::ChatMessage;
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const DB_FILE: &str = "conversations";

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
    /// Monotonic session-id counter — survives deletes. Without it
    /// `next_id` would reuse the id of a just-deleted session, and
    /// `root:id`-keyed frontend state could resurface as a "restored"
    /// session.
    #[serde(default)]
    next_id: i64,
}

/// Per-workspace session store — one row namespace (`<ws>/…`) inside the
/// shared `sessions.db`.
pub struct SessionStore {
    ws: String,
    db: Arc<SessionDatabase>,
}

/// Stable per-workspace key — xxh3 of the canonical root. The office
/// pseudo-workspace keeps a literal `office` key instead, so its sessions
/// sit at `metas/office/<id>` — the same `office/<id>` layout the UI
/// describes (`~/.local/share/husk/office/<id>`).
pub fn workspace_key(root: &Path) -> String {
    if is_office_root(root) {
        return "office".into();
    }
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let h = xxhash_rust::xxh3::xxh3_64(canon.to_string_lossy().as_bytes());
    format!("{h:016x}")
}

/// The host's data directory.
///
/// The session database — and the global preferences that live beside it — are
/// process-wide singletons, so this cannot be a per-call argument: the host
/// sets it once at startup ([`init_data_dir`], called from
/// `SessionManager::spawn_at`). Directory resolution and any rename migration
/// belong to the product, which knows its own name.
static DATA_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Point the store at the host's data directory. Idempotent per directory; a
/// later call replaces the value so a manager booted at another root sees its
/// own. The database itself is opened lazily on first use.
pub fn init_data_dir(dir: PathBuf) {
    if let Ok(mut w) = DATA_DIR.write() {
        *w = Some(dir);
    }
}

/// The data directory the host supplied, if any.
///
/// A test binary gets one throwaway directory for the whole process instead:
/// the store is a process-wide singleton, so parallel tests must agree on the
/// directory or they would clobber each other's value.
pub fn app_data_dir() -> Option<PathBuf> {
    if is_test_binary() {
        return Some(std::env::temp_dir().join(format!(
            "agent-test-{}-data",
            std::process::id()
        )));
    }
    DATA_DIR.read().ok().and_then(|d| d.clone())
}

/// The office-mode pseudo-workspace root: `~/.local/share/husk/office`.
/// Every office session shares this directory as its workspace root, so a
/// document one session produced is readable/writable by all the others
/// (the `resolve` sandbox only confines paths to the workspace root).
/// Created lazily — `SessionManager::switch_workspace` makes it on the
/// first switch.
pub fn office_root() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("office"))
}

/// Whether `root` is the office pseudo-workspace. Compared both raw and
/// canonicalized — the dir may not exist yet on a first switch, which
/// makes `canonicalize` fail on one or both sides.
pub fn is_office_root(root: &Path) -> bool {
    let Some(office) = office_root() else {
        return false;
    };
    if root == office {
        return true;
    }
    let r = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let o = office.canonicalize().unwrap_or(office);
    r == o
}

/// Open (creating) the sessions database at `path`, running the legacy
/// file import once. `path.parent()` is the data dir — the legacy
/// `sessions/` tree and the global `*.json` files live next to the db.
fn open_db(path: &Path) -> std::io::Result<Arc<SessionDatabase>> {
    let (directory, source) = if path.is_file() {
        (
            path.with_extension("jsonl-sessions"),
            Some(path.to_path_buf()),
        )
    } else {
        let source = path
            .parent()
            .map(|p| p.join("sessions.db"))
            .filter(|p| p.is_file());
        (path.to_path_buf(), source)
    };
    let db = SessionDatabase::open(&directory)?;
    migration::import(
        &db,
        directory.parent().unwrap_or(Path::new(".")),
        source.as_deref(),
    )?;
    Ok(db)
}

/// `cargo test` binaries live under `target/*/deps/`; the app binary sits one
/// level up. `#[cfg(test)]` only reaches unit tests inside this lib — `tests/`
/// binaries link us as an external dep — so a runtime marker is the only way
/// BOTH kinds share the same rule: test code must never touch (or lock-fight
/// a running husk over) the real `sessions.db`/`memory.db`.
pub(crate) fn is_test_binary() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|d| d.file_name()).map(|n| n == "deps"))
        .unwrap_or(false)
}

/// The db file the shared handle binds — the env override and test-binary
/// temp copy mirror `global_db`'s own rules so diagnostics name the right
/// file.
pub(crate) fn sessions_db_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AGENT_SESSION_DB") {
        Some(std::path::PathBuf::from(p))
    } else if is_test_binary() {
        Some(std::env::temp_dir().join(format!(
            "agent-test-{}-jsonl-sessions",
            std::process::id()
        )))
    } else {
        app_data_dir().map(|d| d.join(DB_FILE))
    }
}

#[derive(Default)]
struct SharedDb {
    db: Option<Arc<SessionDatabase>>,
    warning: Option<String>,
}

fn shared_db() -> &'static Mutex<SharedDb> {
    static DB: OnceLock<Mutex<SharedDb>> = OnceLock::new();
    DB.get_or_init(|| Mutex::new(SharedDb::default()))
}

fn open_persistent_db() -> std::io::Result<Arc<SessionDatabase>> {
    let path = sessions_db_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法确定会话数据库的数据目录")
    })?;
    open_db(&path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("打开 {} 失败: {error}", path.display()),
        )
    })
}

fn try_global_db() -> std::io::Result<Arc<SessionDatabase>> {
    let mut shared = shared_db().lock().map_err(io_err)?;
    if let Some(db) = &shared.db {
        return Ok(db.clone());
    }
    let db = open_persistent_db()?;
    // Cache successful opens only: a lock or path failure can be retried.
    shared.db = Some(db.clone());
    Ok(db)
}

fn global_db() -> Option<Arc<SessionDatabase>> {
    try_global_db().ok()
}

/// Present on every workspace while the app uses a transient database.
/// Exposed through workspace_info so startup notices cannot be lost before
/// the frontend's event subscription is ready.
pub fn session_storage_warning() -> Option<String> {
    shared_db().lock().ok().and_then(|db| db.warning.clone())
}

/// Run the same validated one-time migration used by app startup. A CLI
/// entry point can surface errors without silently entering temporary mode.
pub fn migrate_session_database() -> std::io::Result<PathBuf> {
    Ok(try_global_db()?.root.clone())
}

// ---------------------------------------------------------------------------
// Legacy import — sessions/<ws>/{index.json,<id>.jsonl,prefs.json,workspace.json,
// <id>.*} plus the three app-level json files, all in ONE write tx so a crash
// leaves nothing half-migrated. Files are renamed aside only after the commit
// succeeds; a failure simply retries on the next open.
// ---------------------------------------------------------------------------

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
    /// Default model for sessions whose workspace saved no model —
    /// a (provider, model) pair validated against `providers` at resolve
    /// time; absent → the config's own `active_provider`/`active_model`.
    /// Mirrors the config.toml field names (`active_*`) on purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    /// See `active_provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(default = "default_pref_permission_mode")]
    pub permission_mode: String,
    #[serde(default = "default_pref_thinking_level")]
    pub thinking_level: Option<String>,
    #[serde(default = "default_pref_agent_mode")]
    pub agent_mode: String,
    /// Which mode the app opens in: "last" (上次使用 — the most recent
    /// workspace, office included) | "build" (编程 — the most recent real
    /// project) | "office" (工作 — the office pseudo-workspace). Read once
    /// at boot by `SessionManager::spawn_at`; it picks the boot workspace,
    /// so nothing downstream consults it.
    #[serde(default = "default_pref_workspace_mode")]
    pub default_workspace_mode: String,
    /// Fraction of the context window that triggers compaction (0.70/0.80/0.90).
    #[serde(default = "default_pref_compact_at")]
    pub compact_at: f32,
    /// Sandbox master switch — absent/true = sandboxed (default);
    /// `false` = process tools run through the loud `none` backend.
    #[serde(default)]
    pub sandbox_enabled: Option<bool>,
    /// Sandbox network override: "auto" (audit decides) | "allow" | "deny".
    #[serde(default)]
    pub sandbox_network: Option<String>,
    /// Per-command sandbox memory cap override (MB).
    #[serde(default)]
    pub sandbox_max_memory_mb: Option<u64>,
    /// Per-command sandbox process-count cap override.
    #[serde(default)]
    pub sandbox_max_processes: Option<u32>,
    /// Extra egress allowlist merged into every sandboxed run's
    /// `allowed_domains` (srt-style entries: `example.com`, `*.example.com`,
    /// `host:443`). The user's standing "these hosts are always fine" list.
    #[serde(default)]
    pub sandbox_allowed_domains: Option<Vec<String>>,
    /// Standing egress denylist — checked before the allowlist.
    #[serde(default)]
    pub sandbox_denied_domains: Option<Vec<String>>,
    /// Memory subsystem master switch — `false` opens no `MemoryStore`: no
    /// `<memory>` block in the prompt and no episode/fact writes.
    #[serde(default = "default_pref_bool_true")]
    pub memory_enabled: bool,
    /// Model distillation — `false` still writes episodes/facts via the
    /// deterministic path but skips the per-turn summarizer model call.
    #[serde(default = "default_pref_bool_true")]
    pub memory_distill: bool,
    /// `.blueprint/` architecture layer — `false` ignores the directory
    /// everywhere (prompt index, `blueprint` tool, turn-end check).
    #[serde(default = "default_pref_bool_true")]
    pub blueprint_enabled: bool,
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

fn default_pref_workspace_mode() -> String {
    "last".into()
}

fn default_pref_thinking_level() -> Option<String> {
    Some("medium".into())
}

impl Default for DefaultPreferences {
    fn default() -> Self {
        Self {
            active_provider: None,
            active_model: None,
            permission_mode: default_pref_permission_mode(),
            thinking_level: default_pref_thinking_level(),
            agent_mode: default_pref_agent_mode(),
            default_workspace_mode: default_pref_workspace_mode(),
            compact_at: default_pref_compact_at(),
            sandbox_enabled: None,
            sandbox_network: None,
            sandbox_max_memory_mb: None,
            sandbox_max_processes: None,
            sandbox_allowed_domains: None,
            sandbox_denied_domains: None,
            memory_enabled: true,
            memory_distill: true,
            blueprint_enabled: true,
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
    files::get(global_db()?.as_ref(), key)
}

fn put_global(key: &str, value: &impl Serialize) -> std::io::Result<()> {
    files::put(try_global_db()?.as_ref(), key, value)
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
    let name = if is_office_root(&canon) {
        // The office pseudo-workspace is a mode, not a folder — display
        // name stays stable instead of reading "office" off the dir.
        "办公".to_string()
    } else {
        canon
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| canon.to_string_lossy().to_string())
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if let Ok(db) = try_global_db() {
        let _ = files::update_global(
            &db,
            "recent_workspaces",
            |list: &mut Vec<RecentWorkspace>| {
                list.retain(|w| w.path != canon && w.path.is_dir());
                list.insert(
                    0,
                    RecentWorkspace {
                        path: canon,
                        name,
                        last_opened: now,
                    },
                );
                list.truncate(15);
            },
        );
    }
}

/// Drop a workspace from recents; session rows are left in the db.
pub fn remove_recent_workspace(workspace_root: &Path) {
    let canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    if let Ok(db) = try_global_db() {
        let _ = files::update_global(
            &db,
            "recent_workspaces",
            |list: &mut Vec<RecentWorkspace>| list.retain(|w| w.path != canon),
        );
    }
}

/// Every workspace that still has session data on disk — the welcome's
/// 「最近打开」source once the sidebar's 项目 list is empty. Removing a
/// project drops its recents record but NOT the store, so this is how a
/// removed project stays one click away.
pub fn known_workspaces() -> Vec<RecentWorkspace> {
    let mut out: Vec<RecentWorkspace> = Vec::new();
    for store in SessionStore::all_workspaces() {
        let Some(row) = store.workspace_row().ok().flatten() else {
            continue;
        };
        let Some(root) = row.root.clone() else {
            continue;
        };
        let path = std::path::PathBuf::from(&root);
        if !path.is_dir() {
            continue;
        }
        let name = if is_office_root(&path) {
            "办公".to_string()
        } else {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| root.clone())
        };
        // No `last_opened` on the workspace row — the newest session's
        // `updated_at` is the closest stamp of real activity.
        let last_opened = store.list().iter().map(|m| m.updated_at).max().unwrap_or(0);
        out.push(RecentWorkspace {
            path,
            name,
            last_opened,
        });
    }
    out.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    out
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
        let db = open_db(&dir.path().join("conversations")).unwrap();
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
            db: open_db(&data.join(DB_FILE)).unwrap(),
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

        // Migration is marked in the file store; original sources remain recoverable.
        assert!(data.join("sessions").exists());
        assert!(!data.join("sessions.legacy").exists());
        assert!(data.join("recent_workspaces.json").exists());
        assert!(!data.join("recent_workspaces.json.bak").exists());
        assert!(!data.join("appearance.json.bak").exists());
    }
}
