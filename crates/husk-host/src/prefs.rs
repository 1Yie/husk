//! Workspace and user preferences — the settings the UI owns.
//!
//! Stored in the session repository's database (`agent-store` global rows), but
//! they are the host's concern, not the kernel's: the repository persists
//! conversations, this module persists the choices that shape them (model,
//! thinking level, permission mode, sandbox / memory / blueprint switches,
//! appearance, recent workspaces). The kernel engine reads none of it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use agent_store::{global_get, global_put, global_update, SessionStore};

use crate::office::is_office_root;

// ---------------------------------------------------------------------------
// Workspace preferences — per-workspace UI prefs (model / thinking / permission)
// persisted in the `prefs` table. Loaded at `SessionManager` boot and fed into
// `SessionConfig`; written whenever the UI changes one of the three selectors.
// ---------------------------------------------------------------------------

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
    /// Global default model for delegated subagents — a (provider, model)
    /// pair. Children whose manifest names no `model:` run on it instead of
    /// the parent session's model; absent → children follow the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_provider: Option<String>,
    /// See `subagent_provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
    /// Global thinking level for delegated subagents. A manifest `thinking:`
    /// still wins; absent here, children inherit the parent session's level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_thinking_level: Option<String>,
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
    /// Seconds a tool approval card may sit unanswered before the kernel
    /// auto-denies it (`0` = wait forever). Matches the kernel's own 300 s.
    #[serde(default = "default_pref_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    /// Seconds an `ask_question` card may sit unanswered before the agent
    /// decides for itself (`0` = wait forever). Matches the kernel's 600 s.
    #[serde(default = "default_pref_question_timeout_secs")]
    pub question_timeout_secs: u64,
    /// Sandbox master switch — `true` = sandboxed; absent/`false` = process
    /// tools run through the loud `none` backend (default off). Rows written
    /// before `schema_version` 1 used absent to mean ON; see
    /// [`migrate_default_preferences`].
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
    /// Semantic recall — score facts against the prompt with a local sentence
    /// encoder instead of the bag-of-hashed-tokens stand-in. Its own switch
    /// because it is the one part of memory that costs a model download; off
    /// by default, and only a build carrying the `semantic` feature honours it.
    #[serde(default)]
    pub semantic_recall: bool,
    /// `.blueprint/` architecture layer — `false` ignores the directory
    /// everywhere (prompt index, `blueprint` tool, turn-end check).
    #[serde(default = "default_pref_bool_true")]
    pub blueprint_enabled: bool,
    /// Desktop (OS) notifications — `false` keeps the notification center
    /// in-app (bell + badge) and pushes nothing to the system shell. An
    /// app-level switch, not session-shaping: no session reads it.
    #[serde(default = "default_pref_bool_true")]
    pub desktop_notifications: bool,
    /// Cue sound for the states that stop the agent loop — a finished turn, a
    /// tool approval or a question waiting on the user, a failure. Separate
    /// from `desktop_notifications`: a window that never leaves the screen
    /// still wants the tone, and a muted desktop may still want it too.
    #[serde(default = "default_pref_bool_true")]
    pub notification_sound: bool,
    /// Row format version. Absent (0) = written before the sandbox default
    /// flipped to off; loading such a row migrates it once.
    #[serde(default)]
    pub schema_version: u32,
}

impl DefaultPreferences {
    /// The subagent default as a whole pair — half a pair means nothing.
    pub fn subagent_pair(&self) -> Option<(String, String)> {
        match (&self.subagent_provider, &self.subagent_model) {
            (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => Some((p.clone(), m.clone())),
            _ => None,
        }
    }
}

/// Current `DefaultPreferences::schema_version`.
pub const DEFAULT_PREFERENCES_VERSION: u32 = 1;

/// Permission mode when neither the workspace nor the stored defaults name
/// one — the single fallback every resolver and the settings echo share.
/// The strictest mode: an untouched install asks before every edit/command.
pub const DEFAULT_PERMISSION_MODE: &str = "default";

/// Seconds an unanswered approval card waits before the kernel auto-denies it
/// — the same 300 s the kernel falls back to.
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 300;

/// Seconds an unanswered `ask_question` card waits before the agent decides
/// on its own — the same 600 s the kernel falls back to.
pub const DEFAULT_QUESTION_TIMEOUT_SECS: u64 = 600;

/// Bring a stored row up to [`DEFAULT_PREFERENCES_VERSION`]; `true` when it
/// changed and must be written back.
///
/// v0 → v1: the sandbox default flipped from on to off. A v0 row only stored
/// `sandbox_enabled` when the user turned it OFF, so absent there meant "on"
/// — pin it, or upgrading would silently drop an existing user's isolation.
pub fn migrate_default_preferences(p: &mut DefaultPreferences) -> bool {
    if p.schema_version >= DEFAULT_PREFERENCES_VERSION {
        return false;
    }
    if p.schema_version < 1 && p.sandbox_enabled.is_none() {
        p.sandbox_enabled = Some(true);
    }
    p.schema_version = DEFAULT_PREFERENCES_VERSION;
    true
}

fn default_pref_bool_true() -> bool {
    true
}

fn default_pref_compact_at() -> f32 {
    0.80
}

/// The kernel's own fallbacks, restated so the settings echo matches what a
/// fresh session does before the user picks a value.
fn default_pref_approval_timeout_secs() -> u64 {
    DEFAULT_APPROVAL_TIMEOUT_SECS
}

fn default_pref_question_timeout_secs() -> u64 {
    DEFAULT_QUESTION_TIMEOUT_SECS
}

fn default_pref_permission_mode() -> String {
    DEFAULT_PERMISSION_MODE.into()
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
            subagent_provider: None,
            subagent_model: None,
            subagent_thinking_level: None,
            permission_mode: default_pref_permission_mode(),
            // No row = no level: the spawn resolvers fall back to `None`,
            // and the settings echo must report the same thing.
            thinking_level: None,
            agent_mode: default_pref_agent_mode(),
            default_workspace_mode: default_pref_workspace_mode(),
            compact_at: default_pref_compact_at(),
            approval_timeout_secs: default_pref_approval_timeout_secs(),
            question_timeout_secs: default_pref_question_timeout_secs(),
            sandbox_enabled: None,
            sandbox_network: None,
            sandbox_max_memory_mb: None,
            sandbox_max_processes: None,
            sandbox_allowed_domains: None,
            sandbox_denied_domains: None,
            memory_enabled: true,
            memory_distill: true,
            semantic_recall: false,
            blueprint_enabled: true,
            desktop_notifications: true,
            notification_sound: true,
            schema_version: DEFAULT_PREFERENCES_VERSION,
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

/// Recently opened workspace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentWorkspace {
    pub path: PathBuf,
    pub name: String,
    pub last_opened: u64,
}

pub fn load_recent_workspaces() -> Vec<RecentWorkspace> {
    let mut list: Vec<RecentWorkspace> = global_get("recent_workspaces").unwrap_or_default();
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

    {
        let _ = global_update("recent_workspaces", |list: &mut Vec<RecentWorkspace>| {
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
        });
    }
}

/// Drop a workspace from recents; session rows are left in the db.
pub fn remove_recent_workspace(workspace_root: &Path) {
    let canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    {
        let _ = global_update("recent_workspaces", |list: &mut Vec<RecentWorkspace>| {
            list.retain(|w| w.path != canon)
        });
    }
}

/// Every workspace that still has session data on disk — the welcome's
/// 「最近打开」source once the sidebar's 项目 list is empty. Removing a
/// project drops its recents record but NOT the store, so this is how a
/// removed project stays one click away.
pub fn known_workspaces() -> Vec<RecentWorkspace> {
    let mut out: Vec<RecentWorkspace> = Vec::new();
    for store in SessionStore::all_workspaces() {
        let Some(path) = store.recorded_root() else {
            continue;
        };
        let root = path.to_string_lossy().to_string();
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
    try_load_default_preferences().unwrap_or_default()
}

/// Load the global default preferences only when a row actually exists —
/// `None` when absent or corrupt. Used as a *fallback* layer below
/// per-workspace prefs so an untouched install keeps the built-in defaults.
pub fn try_load_default_preferences() -> Option<DefaultPreferences> {
    let mut p: DefaultPreferences = global_get("default_preferences")?;
    if migrate_default_preferences(&mut p) {
        let _ = save_default_preferences(&p);
    }
    Some(p)
}

pub fn save_default_preferences(prefs: &DefaultPreferences) -> std::io::Result<()> {
    global_put("default_preferences", prefs)
}

/// A per-install secret mixed into every conversation's routing key. Session
/// ids are small integers and workspace paths are guessable, so a key derived
/// from them alone could be brute-forced back to the session it names; the salt
/// makes the keys on the wire unlinkable to anything outside this install.
///
/// Stored in a global row so a restart keeps every conversation's key (and the
/// gateway's prompt cache with it). When the store is not reachable the salt
/// lives for the process only.
pub fn routing_salt() -> String {
    static PERSISTED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static PROCESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(salt) = PERSISTED.get() {
        return salt.clone();
    }
    if let Some(salt) = load_or_create_salt() {
        return PERSISTED.get_or_init(|| salt).clone();
    }
    PROCESS.get_or_init(random_hex).clone()
}

fn load_or_create_salt() -> Option<String> {
    let fresh = random_hex();
    let mut chosen = None;
    global_update("routing_salt", |stored: &mut String| {
        if stored.is_empty() {
            *stored = fresh.clone();
        }
        chosen = Some(stored.clone());
    })
    .ok()?;
    chosen
}

fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        use std::hash::{BuildHasher, Hasher};
        for chunk in bytes.chunks_mut(8) {
            let word = std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish();
            chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn load_appearance_settings() -> AppearanceSettings {
    global_get("appearance").unwrap_or_default()
}

pub fn save_appearance_settings(s: &AppearanceSettings) -> std::io::Result<()> {
    global_put("appearance", s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One salt per process and install: every conversation key reads the same
    /// secret, and it is long enough that it cannot be guessed back.
    #[test]
    fn the_routing_salt_is_stable_and_unguessable() {
        let salt = routing_salt();
        assert_eq!(salt, routing_salt());
        assert!(
            salt.len() >= 32 && salt.chars().all(|c| c.is_ascii_hexdigit()),
            "{salt}"
        );
        assert_ne!(random_hex(), random_hex());
    }

    /// The identity carries the salt, so two installs name the same session
    /// differently, while one install names it the same way every time.
    #[test]
    fn a_conversation_key_carries_the_install_salt() {
        let ws = std::env::temp_dir();
        let key = crate::session::conversation_key(7, &ws);
        assert!(key.starts_with(&format!("{}:7:", routing_salt())), "{key}");
        assert_eq!(key, crate::session::conversation_key(7, &ws));
        assert_ne!(key, crate::session::conversation_key(8, &ws));
    }

    /// A row saved before the sandbox default flipped stored nothing for a
    /// user who left it ON — loading it must keep them sandboxed.
    #[test]
    fn v0_row_without_sandbox_flag_migrates_to_on() {
        let mut p: DefaultPreferences = serde_json::from_value(serde_json::json!({
            "permission_mode": "acceptEdits",
        }))
        .unwrap();
        assert_eq!(p.schema_version, 0);
        assert!(migrate_default_preferences(&mut p));
        assert_eq!(p.sandbox_enabled, Some(true));
        assert_eq!(p.schema_version, DEFAULT_PREFERENCES_VERSION);
        // Idempotent: a migrated row is left alone.
        assert!(!migrate_default_preferences(&mut p));
    }

    /// A row written before the notification switch existed stores nothing for
    /// it — loading such a row must leave the shell's notifications ON, not
    /// read the missing field as "off".
    #[test]
    fn row_without_notification_flag_defaults_to_on() {
        let p: DefaultPreferences = serde_json::from_value(serde_json::json!({
            "permission_mode": "acceptEdits",
            "schema_version": DEFAULT_PREFERENCES_VERSION,
        }))
        .unwrap();
        assert!(p.desktop_notifications);
    }

    /// An explicit choice in a v0 row survives the migration.
    #[test]
    fn v0_row_with_sandbox_off_stays_off() {
        let mut p: DefaultPreferences =
            serde_json::from_value(serde_json::json!({ "sandbox_enabled": false })).unwrap();
        migrate_default_preferences(&mut p);
        assert_eq!(p.sandbox_enabled, Some(false));
    }

    /// A current row with no flag means the user never turned it on — off.
    #[test]
    fn current_row_without_sandbox_flag_stays_off() {
        let mut p = DefaultPreferences::default();
        assert!(!migrate_default_preferences(&mut p));
        assert_eq!(p.sandbox_enabled, None);
    }

    /// A row written before the semantic switch existed must read as off: the
    /// model is a ~250 MB download, so an upgrade must not start one.
    #[test]
    fn a_row_without_the_semantic_switch_reads_as_off() {
        let p: DefaultPreferences = serde_json::from_value(serde_json::json!({
            "schema_version": DEFAULT_PREFERENCES_VERSION,
        }))
        .unwrap();
        assert!(!p.semantic_recall);
        // Saving it back leaves the switch off rather than dropping the field.
        let round: DefaultPreferences = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert!(!round.semantic_recall);
    }

    /// A row written before the subagent default existed leaves children on
    /// the parent's model.
    #[test]
    fn row_without_subagent_model_follows_the_parent() {
        let p: DefaultPreferences = serde_json::from_value(serde_json::json!({
            "schema_version": DEFAULT_PREFERENCES_VERSION,
        }))
        .unwrap();
        assert_eq!(p.subagent_pair(), None);
    }

    /// The settings echo for an untouched install must match what a new
    /// session actually spawns with.
    #[test]
    fn untouched_defaults_match_the_spawn_fallbacks() {
        let d = DefaultPreferences::default();
        assert_eq!(d.permission_mode, DEFAULT_PERMISSION_MODE);
        assert_eq!(d.thinking_level, None);
    }
}
