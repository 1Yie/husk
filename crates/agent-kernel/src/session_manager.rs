//! `session_manager` — multi-session kernel wiring, frontend-agnostic.
//!
//! Shared by the iced shell (`app-desktop`) and the Tauri shell
//! (`app-tauri`). Each session owns a `SessionActor` on its own `kernel-rt`
//! thread — a background session's turn keeps running when you switch away
//! (product rule: switching never kills the turn). Every actor's `UiEvent`
//! stream is forwarded to ONE `std::sync::mpsc` tagged with its
//! `SessionId`; the frontend drains it however its event loop prefers.
//!
//! Persistence: `SessionStore` (per-workspace) snapshots history at turn
//! boundaries; `open_session` resumes a session from its last snapshot.

use std::collections::HashMap;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use agent_ipc::UiEvent;
use crate::session::{SessionActor, SessionConfig};
use crate::session_store::{SessionMeta, SessionStore};
use agent_llm::{AppConfig, ProviderFactory};
use serde::{Deserialize, Serialize};

/// Detailed model description exposed to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDetails {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub thinking_level_map: Option<HashMap<String, Option<String>>>,
    #[serde(default)]
    pub available_levels: Vec<String>,
}

/// Aggregated model information for UI dropdowns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionModelInfo {
    pub active_provider: String,
    pub active_model: String,
    #[serde(default)]
    pub active_thinking_level: Option<String>,
    #[serde(default)]
    pub active_permission_mode: Option<String>,
    pub config_path: Option<String>,
    pub models: Vec<ModelDetails>,
}

/// A session's live handles — the actor runs on its own thread; these are
/// the UI's write endpoints into it.
pub struct SessionHandle {
    #[allow(dead_code)]
    pub id: i64,
    pub cmd_tx: tokio::sync::mpsc::Sender<agent_ipc::UiCommand>,
    pub decision: Arc<Mutex<Option<(u64, bool)>>>,
    pub permissions: Arc<std::sync::RwLock<crate::permissions::PermissionGate>>,
    /// The session's live thinking level — the engine's shared slot. Read by
    /// `model_info` so the UI shows the session's actual value, which may
    /// differ from the stored default after a prefs change.
    pub thinking_level: Arc<std::sync::RwLock<Option<String>>>,
    pub steer_tx: tokio::sync::mpsc::Sender<String>,
    /// Direct cancel flag — set `true` to abort the in-flight turn. Bypasses
    /// the command pump so a Cancel isn't queued behind `run_turn`.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Sidebar preview — updated from events + turn boundaries.
    pub preview: String,
    /// True while this session's actor is mid-turn (drives the ⟳ marker).
    pub running: bool,
}

/// The multi-session kernel manager — frontend-agnostic. Owns all live
/// actors' handles + the tagged event queue; the frontend keeps
/// `active_id` and renders that session's stream.
pub struct SessionManager {
    store: Arc<SessionStore>,
    provider_cfg: AppConfig,
    handles: HashMap<i64, SessionHandle>,
    /// The globally-tagged event queue every session's forwarder feeds.
    event_tx: std_mpsc::Sender<(i64, UiEvent)>,
    /// Sidebar metadata (persisted index + live preview overrides).
    pub metas: Vec<SessionMeta>,
    /// The session the stream is showing.
    pub active_id: i64,
    /// Active provider + model names — surfaced on the status bar.
    pub provider_name: String,
    pub model_name: String,
    /// Active thinking intensity level across sessions (e.g. "medium", "high").
    pub active_thinking_level: Option<String>,
    /// Active permission mode across sessions (e.g. "default", "acceptEdits", "auto").
    pub permission_mode: String,
    /// Workspace root (for git branch / cwd display).
    pub workspace_root: std::path::PathBuf,
}

impl SessionManager {
    /// Mutable access to a session's live handle (sidebar running/preview).
    pub fn handle_mut(&mut self, id: i64) -> Option<&mut SessionHandle> {
        self.handles.get_mut(&id)
    }

    /// Boot: open the store, resume-or-create the first session, spawn its
    /// actor + forwarder.
    /// Boot the manager + return its `(session_id, UiEvent)` receiver.
    ///
    /// The receiver is the frontend's event tap — the iced shell drains it
    /// on `Tick`, the Tauri shell forwards it into `app.emit`. It's split
    /// out (rather than held on `self`) so the frontend can move it into a
    /// forwarder thread without partially moving the manager.
    pub fn spawn() -> (Self, std_mpsc::Receiver<(i64, UiEvent)>) {
        Self::spawn_at(None)
    }

    /// Boot the manager at a specific workspace root (or current directory / most recent).
    pub fn spawn_at(root: Option<std::path::PathBuf>) -> (Self, std_mpsc::Receiver<(i64, UiEvent)>) {
        let cwd = root
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
        let cfg = AppConfig::load(None).unwrap_or_default();
        let store = Arc::new(SessionStore::open(&canon).unwrap_or_else(|_| {
            // Fallback: temp dir so the app still boots without a data dir.
            SessionStore::open(std::path::Path::new("/tmp"))
                .expect("session store")
        }));
        let (event_tx, event_rx) = std_mpsc::channel();
        let (_p, default_model, default_pname) = resolve_provider(&cfg);

        crate::session_store::record_recent_workspace(&canon);

        let prefs = store.load_prefs();
        // Global user defaults — the settings window's stored preference —
        // fill any gap the workspace prefs leave. Absent file → built-ins.
        let defaults = crate::session_store::try_load_default_preferences();
        let (provider_name, model_name) = match (&prefs.provider, &prefs.model) {
            (Some(p), Some(m)) if cfg.providers.contains_key(p) => (p.clone(), m.clone()),
            _ => (default_pname, default_model),
        };
        let permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| "default".into());
        let active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.and_then(|d| d.thinking_level));

        let mut mgr = Self {
            metas: store.list(),
            store,
            provider_cfg: cfg,
            handles: HashMap::new(),
            event_tx,
            active_id: 0,
            provider_name,
            model_name,
            active_thinking_level,
            permission_mode,
            workspace_root: canon,
        };

        // Resume the most recent session if one exists; else start fresh.
        let first = mgr.metas.first().map(|m| m.id);
        match first {
            Some(id) => mgr.open_session(id),
            None => mgr.new_session(),
        }
        (mgr, event_rx)
    }

    /// Switch to another workspace directory: opens that workspace's session store,
    /// sets active workspace, records in recents, and loads or creates its session.
    pub fn switch_workspace(&mut self, new_root: std::path::PathBuf) -> std::io::Result<()> {
        let canon = new_root.canonicalize().unwrap_or_else(|_| new_root.clone());
        let store = Arc::new(SessionStore::open(&canon)?);
        let prefs = store.load_prefs();
        let defaults = crate::session_store::try_load_default_preferences();
        let (_p, default_model, default_pname) = resolve_provider(&self.provider_cfg);
        let (provider_name, model_name) = match (&prefs.provider, &prefs.model) {
            (Some(p), Some(m)) if self.provider_cfg.providers.contains_key(p) => (p.clone(), m.clone()),
            _ => (default_pname, default_model),
        };
        self.permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| "default".into());
        self.active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.and_then(|d| d.thinking_level));
        self.provider_name = provider_name;
        self.model_name = model_name;

        self.workspace_root = canon;
        self.store = store;
        self.metas = self.store.list();
        self.handles.clear();
        crate::session_store::record_recent_workspace(&self.workspace_root);

        let first = self.metas.first().map(|m| m.id);
        match first {
            Some(id) => self.open_session(id),
            None => self.new_session(),
        }
        Ok(())
    }

    /// List recent workspaces from the persistent recent_workspaces store.
    pub fn recent_workspaces(&self) -> Vec<crate::session_store::RecentWorkspace> {
        crate::session_store::load_recent_workspaces()
    }

    /// Spawn an actor for `id` (resume from store if a snapshot exists).
    fn spawn_actor(&mut self, id: i64) {
        let provider = self
            .provider_cfg
            .providers
            .get(&self.provider_name)
            .and_then(|pcfg| ProviderFactory::build(pcfg).ok())
            .unwrap_or_else(|| {
                let (p, _, _) = resolve_provider(&self.provider_cfg);
                p
            });
        let model = self.model_name.clone();
        let mentry = self
            .provider_cfg
            .providers
            .get(&self.provider_name)
            .and_then(|p| p.find_model(&model));
        let thinking_level_map = mentry
            .and_then(|m| m.detailed())
            .and_then(|d| d.thinking_level_map.clone());

        let cfg = SessionConfig {
            workspace_root: self.workspace_root.clone(),
            provider,
            model,
            temperature: 1.0,
            permission_mode: self.permission_mode.clone(),
            track_dirty: true,
            thinking_level: self.active_thinking_level.clone(),
            thinking_level_map,
        };

        let (mut actor, channels) = match self.store.load_history(id) {
            Some(hist) => SessionActor::resume(cfg, id, hist),
            // Fresh session — caller-assigned id so store + sidebar agree.
            None => SessionActor::spawn_with_id(cfg, id),
        };

        let cmd_tx = actor.command_sender();
        let decision = actor.decision_writer();
        let permissions = actor.permissions_writer();
        let thinking_level = actor.thinking_writer();
        let steer_tx = actor.steer_writer();
        let cancel = actor.cancel_writer();

        // Actor run() on its own tokio thread — a background session keeps
        // running its turn while the UI shows another session.
        std::thread::Builder::new()
            .name(format!("session-{id}-rt"))
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async move { actor.run().await });
            })
            .expect("spawn session rt");

        // Forward this session's events into the tagged global queue.
        {
            let mut rx = channels.event_rx;
            let tx = self.event_tx.clone();
            std::thread::Builder::new()
                .name(format!("session-{id}-fwd"))
                .spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
                    rt.block_on(async move {
                        while let Some(ev) = rx.recv().await {
                            if tx.send((id, ev)).is_err() {
                                break;
                            }
                        }
                    });
                })
                .expect("spawn session fwd");
        }

        let preview = self
            .metas
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.preview.clone())
            .unwrap_or_else(|| "new session".into());
        self.handles.insert(id, SessionHandle {
            id,
            cmd_tx,
            decision,
            permissions,
            thinking_level,
            steer_tx,
            cancel,
            preview,
            running: false,
        });
    }

    /// Create a brand-new session (fresh actor, no history) and activate it.
    pub fn new_session(&mut self) {
        let id = self.store.next_id();
        // Reserve the id in the index so the sidebar lists it immediately.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = self.store.upsert_meta(SessionMeta {
            id,
            title: "新会话".into(),
            preview: String::new(),
            updated_at: now,
        });
        self.metas = self.store.list();
        self.spawn_actor(id);
        self.active_id = id;
    }

    /// Switch to an existing session — spawns its actor (resumed from the
    /// store) if not already live; the outgoing actor keeps running.
    pub fn open_session(&mut self, id: i64) {
        if !self.handles.contains_key(&id) {
            self.spawn_actor(id);
        }
        self.active_id = id;
    }

    /// Delete a session — abort its turn, drop the actor handle (closing
    /// the command channel ends its loop), and remove the store data.
    /// If it was active, switch to the most recent remaining session, or
    /// open a fresh one when none are left.
    pub fn delete_session(&mut self, id: i64) {
        if let Some(h) = self.handles.remove(&id) {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = self.store.remove(id);
        self.metas = self.store.list();
        if self.active_id == id {
            match self.metas.first().map(|m| m.id) {
                Some(next) => self.open_session(next),
                None => self.new_session(),
            }
        }
    }

    /// Fork — copy the latest persisted snapshot into a fresh session and
    /// activate it. Returns the new id; `None` when the source session has
    /// no history to copy yet.
    pub fn fork_session(&mut self, id: i64) -> Option<i64> {
        let hist = self.store.load_history(id)?;
        let new_id = self.store.next_id();
        // Snapshot first — spawn_actor() resumes from the store, so the
        // new actor must already see the forked history when it starts.
        self.store.snapshot(new_id, &hist).ok()?;
        let src = self.metas.iter().find(|m| m.id == id).cloned();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = self.store.upsert_meta(SessionMeta {
            id: new_id,
            title: src
                .as_ref()
                .map(|m| format!("{} · 副本", m.title))
                .unwrap_or_else(|| "新会话".into()),
            preview: src.map(|m| m.preview).unwrap_or_default(),
            updated_at: now,
        });
        self.metas = self.store.list();
        self.spawn_actor(new_id);
        self.active_id = new_id;
        Some(new_id)
    }

    /// The active session's handles (for UI commands).
    pub fn active(&self) -> Option<&SessionHandle> {
        self.handles.get(&self.active_id)
    }

    /// Load a session's persisted history (for rebuilding a closed view).
    pub fn store_history(&self, id: i64) -> Option<Vec<agent_llm::types::ChatMessage>> {
        self.store.load_history(id)
    }

    /// Sidebar rows — persisted metas overlaid with live running/preview.
    pub fn sidebar_rows(&self) -> Vec<(i64, String, String, bool, bool)> {
        self.metas
            .iter()
            .map(|m| {
                let live = self.handles.get(&m.id);
                let preview = live.map(|h| h.preview.clone()).unwrap_or_else(|| m.preview.clone());
                let running = live.map(|h| h.running).unwrap_or(false);
                (m.id, m.title.clone(), preview, m.id == self.active_id, running)
            })
            .collect()
    }

    /// Model info (active provider, active model, active thinking level, configured models list, config path).
    pub fn model_info(&mut self) -> SessionModelInfo {
        // Hot-reload config from disk to pick up any changes in config.toml
        if let Ok(cfg) = AppConfig::load(None) {
            self.provider_cfg = cfg;
        }

        let mut models = Vec::new();
        for (pname, pcfg) in &self.provider_cfg.providers {
            if !pcfg.models.is_empty() {
                for m in &pcfg.models {
                    let d = m.detailed();
                    let reasoning = d.and_then(|x| x.reasoning).unwrap_or(false)
                        || d.and_then(|x| x.thinking_level_map.as_ref()).is_some();
                    let thinking_level_map = d.and_then(|x| x.thinking_level_map.clone());
                    let mut available_levels = Vec::new();
                    if let Some(map) = &thinking_level_map {
                        for lvl in &["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
                            if *lvl == "off" {
                                available_levels.push("off".to_string());
                            } else if let Some(Some(_)) = map.get(*lvl) {
                                available_levels.push(lvl.to_string());
                            }
                        }
                    } else if reasoning {
                        available_levels = vec![
                            "off".into(),
                            "low".into(),
                            "medium".into(),
                            "high".into(),
                            "max".into(),
                        ];
                    }

                    models.push(ModelDetails {
                        provider: pname.clone(),
                        model: m.id().to_string(),
                        name: Some(m.name().to_string()),
                        reasoning,
                        thinking_level_map,
                        available_levels,
                    });
                }
            } else if let Some(dm) = &pcfg.default_model {
                models.push(ModelDetails {
                    provider: pname.clone(),
                    model: dm.clone(),
                    name: Some(dm.clone()),
                    reasoning: false,
                    thinking_level_map: None,
                    available_levels: Vec::new(),
                });
            } else {
                models.push(ModelDetails {
                    provider: pname.clone(),
                    model: self.model_name.clone(),
                    name: Some(self.model_name.clone()),
                    reasoning: false,
                    thinking_level_map: None,
                    available_levels: Vec::new(),
                });
            }
        }

        if !models.iter().any(|m| m.provider == self.provider_name && m.model == self.model_name) {
            models.insert(0, ModelDetails {
                provider: self.provider_name.clone(),
                model: self.model_name.clone(),
                name: Some(self.model_name.clone()),
                reasoning: false,
                thinking_level_map: None,
                available_levels: Vec::new(),
            });
        }

        if self.active_thinking_level.is_none() {
            if let Some(m) = models.iter().find(|m| m.provider == self.provider_name && m.model == self.model_name) {
                if m.reasoning {
                    if m.available_levels.contains(&"medium".to_string()) {
                        self.active_thinking_level = Some("medium".into());
                    } else if let Some(first_non_off) = m.available_levels.iter().find(|l| *l != "off") {
                        self.active_thinking_level = Some(first_non_off.clone());
                    } else {
                        self.active_thinking_level = Some("off".into());
                    }
                }
            }
        }

        // Report the ACTIVE session's live values — the composer mirrors the
        // session's actual gate/level, which may differ from the stored
        // defaults once the settings popup changes them (prefs only apply to
        // sessions spawned afterwards).
        let (live_mode, live_level) = self
            .active()
            .map(|h| {
                let m = h
                    .permissions
                    .read()
                    .ok()
                    .map(|g| g.mode().as_str().to_string());
                let t = h.thinking_level.read().ok().and_then(|l| l.clone());
                (m, t)
            })
            .unwrap_or((None, None));

        let path = AppConfig::default_path().map(|p| p.to_string_lossy().to_string());
        SessionModelInfo {
            active_provider: self.provider_name.clone(),
            active_model: self.model_name.clone(),
            active_thinking_level: live_level.or_else(|| self.active_thinking_level.clone()),
            active_permission_mode: live_mode.or_else(|| Some(self.permission_mode.clone())),
            config_path: path,
            models,
        }
    }

    /// The stored default preferences — what NEW sessions spawn with.
    /// The settings popup reads/writes these; already-running sessions keep
    /// whatever they were spawned (or later switched) with.
    pub fn default_prefs(&self) -> (String, Option<String>) {
        (self.permission_mode.clone(), self.active_thinking_level.clone())
    }

    /// Update the stored default preferences — applies to sessions spawned
    /// AFTER this call; live sessions are untouched (no gate write, no
    /// UiCommand, no SystemMessage in their stream).
    pub fn set_default_prefs(&mut self, permission_mode: Option<String>, thinking_level: Option<String>) {
        if let Some(m) = permission_mode {
            self.permission_mode = m;
        }
        if let Some(l) = thinking_level {
            self.active_thinking_level = if l.is_empty() { None } else { Some(l) };
        }
        self.persist_prefs();
        // Also persist the global defaults so fresh workspaces inherit them.
        let _ = crate::session_store::save_default_preferences(
            &crate::session_store::DefaultPreferences {
                permission_mode: self.permission_mode.clone(),
                thinking_level: self.active_thinking_level.clone(),
            },
        );
    }

    /// Persist current workspace composer preferences (model, thinking level, permission mode).
    pub fn persist_prefs(&self) {
        let prefs = crate::session_store::WorkspacePrefs {
            provider: Some(self.provider_name.clone()),
            model: Some(self.model_name.clone()),
            thinking_level: self.active_thinking_level.clone(),
            permission_mode: Some(self.permission_mode.clone()),
        };
        let _ = self.store.save_prefs(&prefs);
    }

    /// Update active model & provider in manager metadata and persist to workspace prefs.
    pub fn set_model(&mut self, provider: String, model: String) {
        self.provider_name = provider;
        self.model_name = model;
        self.persist_prefs();
    }

    /// Update active thinking level in manager metadata and persist to workspace prefs.
    pub fn set_thinking_level(&mut self, level: String) {
        self.active_thinking_level = if level.is_empty() { None } else { Some(level) };
        self.persist_prefs();
    }

    /// Update active permission mode in manager metadata and persist to workspace prefs.
    pub fn set_permission_mode(&mut self, mode: String) {
        self.permission_mode = mode;
        self.persist_prefs();
    }
}

/// Resolve the active provider from `config.toml`.
fn resolve_provider(
    cfg: &AppConfig,
) -> (Arc<dyn agent_llm::LlmProvider>, String, String) {
    if let Some(name) = &cfg.active_provider {
        if let Some(pcfg) = cfg.providers.get(name) {
            if let Ok(p) = ProviderFactory::build(pcfg) {
                let model = cfg
                    .active_model
                    .clone()
                    .or_else(|| pcfg.default_model.clone())
                    .unwrap_or_else(|| "default".into());
                return (p, model, name.clone());
            }
        }
    }
    for (name, pcfg) in &cfg.providers {
        if let Ok(p) = ProviderFactory::build(pcfg) {
            let model = cfg
                .active_model
                .clone()
                .or_else(|| pcfg.default_model.clone())
                .unwrap_or_else(|| "default".into());
            return (p, model, name.clone());
        }
    }
    (
        Arc::new(agent_llm::adapters::MockProvider::new()),
        "mock".into(),
        "mock (no provider configured)".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_with_devin_swe2() {
        let (mut mgr, _rx) = SessionManager::spawn();
        let info = mgr.model_info();
        assert_eq!(info.active_provider, "devin");
        assert_eq!(info.active_model, "devin/swe-2");
        assert_eq!(info.active_thinking_level.as_deref(), Some("medium"));
        assert_eq!(info.active_permission_mode.as_deref(), Some("default"));
        let swe2 = info
            .models
            .iter()
            .find(|m| m.model == "devin/swe-2")
            .expect("swe-2 model found");
        assert!(swe2.reasoning);
        assert_eq!(swe2.available_levels, vec!["off", "medium", "high", "max"]);
    }

    #[test]
    fn test_workspace_prefs_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let (mut mgr, _rx) = SessionManager::spawn_at(Some(path.clone()));
            mgr.set_model("devin".into(), "devin/swe-2".into());
            mgr.set_thinking_level("high".into());
            mgr.set_permission_mode("acceptEdits".into());
        }
        // Spawn a fresh manager on the same directory — should restore preferences from prefs.json
        let (mut mgr2, _rx) = SessionManager::spawn_at(Some(path));
        assert_eq!(mgr2.provider_name, "devin");
        assert_eq!(mgr2.model_name, "devin/swe-2");
        assert_eq!(mgr2.active_thinking_level.as_deref(), Some("high"));
        assert_eq!(mgr2.permission_mode, "acceptEdits");
        let info = mgr2.model_info();
        assert_eq!(info.active_permission_mode.as_deref(), Some("acceptEdits"));
        assert_eq!(info.active_thinking_level.as_deref(), Some("high"));
    }
}
