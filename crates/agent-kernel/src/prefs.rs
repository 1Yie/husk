//! Workspace and user preferences — the settings the UI owns.
//!
//! These live beside the session repository because they are stored in the same
//! database, but they are a different concern: the repository persists
//! conversations, this module persists the choices that shape them (model,
//! thinking level, permission mode, appearance, recent workspaces). Splitting
//! them is what lets the repository be extracted on its own.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session_store::{global_get, global_put, global_update, is_office_root, SessionStore};

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
        let _ = global_update(
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
    {
        let _ = global_update(
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
    global_get("default_preferences").unwrap_or_default()
}

/// Load the global default preferences only when a row actually exists —
/// `None` when absent or corrupt. Used as a *fallback* layer below
/// per-workspace prefs so an untouched install keeps the built-in defaults.
pub fn try_load_default_preferences() -> Option<DefaultPreferences> {
    global_get("default_preferences")
}

pub fn save_default_preferences(prefs: &DefaultPreferences) -> std::io::Result<()> {
    global_put("default_preferences", prefs)
}

pub fn load_appearance_settings() -> AppearanceSettings {
    global_get("appearance").unwrap_or_default()
}

pub fn save_appearance_settings(s: &AppearanceSettings) -> std::io::Result<()> {
    global_put("appearance", s)
}
