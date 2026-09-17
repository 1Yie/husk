//! `bridge` — kernel wiring for the iced shell, multi-session edition.
//!
//! Each session owns a `SessionActor` on its own `kernel-rt` thread — a
//! background session's turn keeps running when you switch away (product
//! rule: switching never kills the turn). Every actor's `UiEvent` stream is
//! forwarded to ONE `std::sync::mpsc` tagged with its `SessionId`; the iced
//! `Tick` subscription drains it and routes by `active_id`.
//!
//! Persistence: `SessionStore` (per-workspace) snapshots history at turn
//! boundaries; `select_session` resumes a session from its last snapshot.

use std::collections::HashMap;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use agent_ipc::UiEvent;
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_kernel::session_store::{SessionMeta, SessionStore};
use agent_llm::{AppConfig, ProviderFactory};

/// A session's live handles — the actor runs on its own thread; these are
/// the UI's write endpoints into it.
pub struct SessionHandle {
    #[allow(dead_code)]
    pub id: i64,
    pub cmd_tx: tokio::sync::mpsc::Sender<agent_ipc::UiCommand>,
    pub decision: Arc<Mutex<Option<bool>>>,
    pub steer_tx: tokio::sync::mpsc::Sender<String>,
    /// Sidebar preview — updated from events + turn boundaries.
    pub preview: String,
    /// True while this session's actor is mid-turn (drives the ⟳ marker).
    pub running: bool,
}

/// The multi-session kernel manager. Owns all live actors' handles; the UI
/// keeps `active_id` and renders that session's stream.
pub struct SessionManager {
    store: Arc<SessionStore>,
    provider_cfg: AppConfig,
    handles: HashMap<i64, SessionHandle>,
    /// The globally-tagged event queue every session's forwarder feeds.
    event_tx: std_mpsc::Sender<(i64, UiEvent)>,
    /// UI-side receiver — `App::update` drains it on `Tick`.
    pub event_rx: std_mpsc::Receiver<(i64, UiEvent)>,
    /// Sidebar metadata (persisted index + live preview overrides).
    pub metas: Vec<SessionMeta>,
    /// The session the stream is showing.
    pub active_id: i64,
    /// Active provider + model names — surfaced on the status bar.
    pub provider_name: String,
    pub model_name: String,
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
    pub fn spawn() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let cfg = AppConfig::load(None).unwrap_or_default();
        let store = Arc::new(SessionStore::open(&cwd).unwrap_or_else(|_| {
            // Fallback: temp dir so the app still boots without a data dir.
            SessionStore::open(std::path::Path::new("/tmp"))
                .expect("session store")
        }));
        let (event_tx, event_rx) = std_mpsc::channel();
        let (_p, model, pname) = resolve_provider(&cfg);

        let mut mgr = Self {
            metas: store.list(),
            store,
            provider_cfg: cfg,
            handles: HashMap::new(),
            event_tx,
            event_rx,
            active_id: 0,
            provider_name: pname,
            model_name: model,
            workspace_root: cwd,
        };

        // Resume the most recent session if one exists; else start fresh.
        let first = mgr.metas.first().map(|m| m.id);
        match first {
            Some(id) => mgr.open_session(id),
            None => mgr.new_session(),
        }
        mgr
    }

    /// Spawn an actor for `id` (resume from store if a snapshot exists).
    fn spawn_actor(&mut self, id: i64) {
        let (provider, model, _label) = resolve_provider(&self.provider_cfg);
        let cfg = SessionConfig {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| ".".into()),
            provider,
            model,
            temperature: 1.0,
            permission_mode: "default".into(),
            track_dirty: true,
        };

        let (mut actor, channels) = match self.store.load_history(id) {
            Some(hist) => SessionActor::resume(cfg, id, hist),
            // Fresh session — caller-assigned id so store + sidebar agree.
            None => SessionActor::spawn_with_id(cfg, id),
        };

        let cmd_tx = actor.command_sender();
        let decision = actor.decision_writer();
        let steer_tx = actor.steer_writer();

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
            steer_tx,
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
            title: "new session".into(),
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
