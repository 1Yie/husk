//! `session_manager` — multi-session kernel wiring, frontend-agnostic.
//!
//! Each session owns a `SessionActor` on its own `kernel-rt` thread, so a
//! background turn keeps running when the user switches away. Every actor's
//! `UiEvent` stream is forwarded into one channel tagged with its `SessionId`.
//! `SessionStore` snapshots history at turn boundaries; `open_session` resumes
//! from the last snapshot.

use std::collections::HashMap;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use crate::channels::{UiSink, UiStatsSnapshot};
use crate::session::{SessionActor, SessionConfig};
use crate::session_store::{SessionMeta, SessionStore};
use agent_ipc::UiEvent;
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
    /// Context window from the model's config entry — `None` means the
    /// engine's 256_000 default applies.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Maximum output tokens (`maxTokens`) — `None` means the adapter's own
    /// default cap applies. Shown in the model picker's detail line.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Pricing ($/1M tokens) from the model's config entry — `None` means
    /// "cost unknown, don't render a meter". The frontend turns the last
    /// `Usage` event into a $ figure against these rates.
    #[serde(default)]
    pub cost: Option<agent_llm::ModelCost>,
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
    /// Active agent mode (`build` | `plan` | `goal`) — the composer's
    /// mode picker reads this on load.
    #[serde(default)]
    pub active_agent_mode: Option<String>,
    pub config_path: Option<String>,
    pub models: Vec<ModelDetails>,
}

/// One conversation row inside a project (sidebar project tree).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSessionRow {
    pub id: i64,
    pub title: String,
    pub preview: String,
    /// Unix seconds of last activity — the sidebar's cross-project "recent"
    /// list merges rows by this key.
    pub updated_at: u64,
    /// Only ever true for the active workspace's active session.
    pub active: bool,
    /// Live turn flag. Only the active workspace has actors to report it,
    /// so rows of other projects always read `false`.
    pub running: bool,
    /// User-pinned rows float to the top of the sidebar list.
    pub pinned: bool,
}

/// One project (workspace) with its full conversation list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOverview {
    pub root: String,
    pub name: String,
    pub last_opened: u64,
    /// This is the kernel's active workspace.
    pub current: bool,
    /// Every persisted conversation, newest first.
    pub sessions: Vec<ProjectSessionRow>,
}

/// Display name for a workspace — its final path component, or the full
/// path when there is none (filesystem root).
fn project_name(root: &std::path::Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

/// A session's live handles — the actor runs on its own thread; these are
/// the UI's write endpoints into it.
pub struct SessionHandle {
    #[allow(dead_code)]
    pub id: i64,
    pub cmd_tx: tokio::sync::mpsc::Sender<agent_ipc::UiCommand>,
    pub decision: Arc<Mutex<Option<(u64, bool)>>>,
    /// `AnswerQuestion` resolves the parked ask_question oneshot — same
    /// bypass-the-pump channel as `decision`.
    pub ask: Arc<crate::tools::registry::AskChannel>,
    pub permissions: Arc<std::sync::RwLock<crate::permissions::PermissionGate>>,
    /// The session's live agent mode — `SetAgentMode` writes here mid-turn
    /// so the next sampling round dispatches against the swapped registry.
    pub agent_mode: Arc<std::sync::RwLock<crate::mode::AgentMode>>,
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
    /// This session's UI event queue — kept here so the shell can read the
    /// delivery counters (coalesced/dropped deltas, queue high-water mark)
    /// without reaching into the actor.
    pub ui: UiSink,
}

/// The multi-session kernel manager — frontend-agnostic. Owns all live
/// actors' handles + the tagged event queue; the frontend keeps
/// `active_id` and renders that session's stream.
pub struct SessionManager {
    store: Arc<SessionStore>,
    provider_cfg: AppConfig,
    /// mtime of the config file at the last load. `None` means "reload on the
    /// next check" — the file was missing, or a reload could not be delivered.
    config_stamp: Option<std::time::SystemTime>,
    handles: HashMap<i64, SessionHandle>,
    /// Live actors of workspaces the user switched AWAY from — parked by
    /// canonical root so a background turn keeps running (and keeps its
    /// command/approval endpoints) until the user returns. Restored into
    /// `handles` on the next `switch_workspace` back. Product rule:
    /// switching never kills the turn — not even across workspaces.
    parked: HashMap<String, HashMap<i64, SessionHandle>>,
    /// The globally-tagged event queue every session's forwarder feeds.
    /// Each envelope is `(workspace_root, session_id, event)` — session
    /// ids are per-workspace, so the root is what keeps a background
    /// workspace's events from colliding with a same-numbered session in
    /// the active one.
    event_tx: std_mpsc::Sender<(String, i64, UiEvent)>,
    /// MCP servers registered once at boot (`agent_plugin::load_all`) — every
    /// session's engine gets this router, so plugin tools are advertised and
    /// dispatchable in the ReAct loop. `None` = no manifest discovered.
    plugins: Option<crate::engine::PluginHandle>,
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
    /// Active agent mode across sessions (`build` | `plan` | `goal`).
    pub agent_mode: String,
    /// Fraction of the context window that triggers compaction — the
    /// settings UI's 70/80/90% choices; applies to actors spawned after set.
    pub compact_at: f32,
    /// Workspace root (for git branch / cwd display).
    pub workspace_root: std::path::PathBuf,
    /// Workspace is open. `false` = empty state: no actors, no sessions.
    pub workspace_active: bool,
}

impl SessionManager {
    /// Mutable access to a session's live handle in the ACTIVE workspace
    /// (sidebar running/preview).
    pub fn handle_mut(&mut self, id: i64) -> Option<&mut SessionHandle> {
        self.handles.get_mut(&id)
    }

    /// Mutable access to a session's live handle wherever it lives — the
    /// event forwarder uses this since a background workspace's actor
    /// keeps emitting after a switch (its handle sits in `parked`).
    /// `root` is the canonical workspace root the event was tagged with.
    pub fn handle_mut_at(&mut self, root: &str, id: i64) -> Option<&mut SessionHandle> {
        if self.workspace_root.to_string_lossy() == root {
            self.handles.get_mut(&id)
        } else {
            self.parked.get_mut(root)?.get_mut(&id)
        }
    }

    /// Boot: open the store, resume-or-create the first session, spawn its
    /// actor + forwarder.
    /// Boot the manager + return its `(session_id, UiEvent)` receiver.
    ///
    /// The receiver is the frontend's event tap — the iced shell drains it
    /// on `Tick`, the Tauri shell forwards it into `app.emit`. It's split
    /// out (rather than held on `self`) so the frontend can move it into a
    /// forwarder thread without partially moving the manager.
    pub fn spawn() -> (Self, std_mpsc::Receiver<(String, i64, UiEvent)>) {
        Self::spawn_at(None)
    }

    /// Boot the manager at a specific workspace root (or current directory / most recent).
    pub fn spawn_at(
        root: Option<std::path::PathBuf>,
    ) -> (Self, std_mpsc::Receiver<(String, i64, UiEvent)>) {
        // No explicit root → MRU recents head. No recents → empty state
        // (no cwd fallback — don't silently attach the launch directory).
        let cwd: Option<std::path::PathBuf> = root.or_else(|| {
            crate::session_store::load_recent_workspaces()
                .first()
                .map(|w| w.path.clone())
                .filter(|p| p.is_dir())
        });
        let cfg = AppConfig::load(None).unwrap_or_default();
        let (event_tx, event_rx) = std_mpsc::channel();
        let (_p, default_model, default_pname) = resolve_provider(&cfg);

        // MCP servers: registered once, before any session exists, so the
        // session resumed below is built with the router. `load_all` bounds
        // each registration (8 s) and skips failures, and an empty plugin dir
        // is skipped without even creating a runtime.
        let plugins = cwd.as_ref().map(|root| {
            let handle: crate::engine::PluginHandle = Arc::new(std::sync::RwLock::new(None));
            *handle.write().unwrap() = Self::load_plugins(root);
            handle
        });
        let plugins_for_manager = plugins;

        // Empty boot: inert store, no actors, no metas.
        let Some(cwd) = cwd else {
            let store = Arc::new(
                SessionStore::open(
                    &crate::session_store::app_data_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp")),
                )
                .expect("session store"),
            );
            let defaults = crate::session_store::try_load_default_preferences();
            if let Some(d) = &defaults {
                crate::sandbox_prefs::set(crate::sandbox_prefs::SandboxLimits {
                    network: d
                        .sandbox_network
                        .as_deref()
                        .and_then(crate::sandbox_prefs::network_from_label),
                    max_memory_mb: d.sandbox_max_memory_mb,
                    max_processes: d.sandbox_max_processes,
                });
            }
            let mgr = Self {
                plugins: plugins_for_manager.clone(),
                metas: Vec::new(),
                store,
                provider_cfg: cfg,
                config_stamp: Self::config_stamp_now(),
                handles: HashMap::new(),
                parked: HashMap::new(),
                event_tx,
                active_id: 0,
                provider_name: default_pname,
                model_name: default_model,
                active_thinking_level: defaults.as_ref().and_then(|d| d.thinking_level.clone()),
                permission_mode: defaults
                    .as_ref()
                    .map(|d| d.permission_mode.clone())
                    .unwrap_or_else(|| "default".into()),
                agent_mode: defaults
                    .as_ref()
                    .map(|d| d.agent_mode.clone())
                    .unwrap_or_else(|| "build".into()),
                compact_at: defaults
                    .as_ref()
                    .map(|d| d.compact_at)
                    .unwrap_or(crate::compaction::COMPACT_AT),
                workspace_root: std::path::PathBuf::new(),
                workspace_active: false,
            };
            return (mgr, event_rx);
        };

        let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
        let store = Arc::new(SessionStore::open(&canon).unwrap_or_else(|_| {
            // Fallback: temp dir so the app still boots without a data dir.
            SessionStore::open(std::path::Path::new("/tmp")).expect("session store")
        }));

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
        let agent_mode = prefs
            .agent_mode
            .or_else(|| defaults.as_ref().map(|d| d.agent_mode.clone()))
            .unwrap_or_else(|| "build".into());
        let active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.as_ref().and_then(|d| d.thinking_level.clone()));
        let compact_at = defaults
            .as_ref()
            .map(|d| d.compact_at)
            .unwrap_or(crate::compaction::COMPACT_AT);
        // Sandbox overrides are process-global — seed the slot once so
        // process tools read them even before the settings window opens.
        if let Some(d) = &defaults {
            crate::sandbox_prefs::set(crate::sandbox_prefs::SandboxLimits {
                network: d
                    .sandbox_network
                    .as_deref()
                    .and_then(crate::sandbox_prefs::network_from_label),
                max_memory_mb: d.sandbox_max_memory_mb,
                max_processes: d.sandbox_max_processes,
            });
        }

        let mut mgr = Self {
            plugins: plugins_for_manager,
            metas: store.list(),
            store,
            provider_cfg: cfg,
            config_stamp: Self::config_stamp_now(),
            handles: HashMap::new(),
            parked: HashMap::new(),
            event_tx,
            active_id: 0,
            provider_name,
            model_name,
            active_thinking_level,
            permission_mode,
            agent_mode,
            compact_at,
            workspace_root: canon,
            workspace_active: true,
        };

        // Resume the session the user last had open; fall back to the
        // most recent when it no longer exists, or start fresh.
        let first = mgr
            .store
            .last_active()
            .filter(|id| mgr.metas.iter().any(|m| m.id == *id))
            .or_else(|| mgr.metas.first().map(|m| m.id));
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
            (Some(p), Some(m)) if self.provider_cfg.providers.contains_key(p) => {
                (p.clone(), m.clone())
            }
            _ => (default_pname, default_model),
        };
        self.permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| "default".into());
        self.agent_mode = prefs
            .agent_mode
            .or_else(|| defaults.as_ref().map(|d| d.agent_mode.clone()))
            .unwrap_or_else(|| "build".into());
        self.active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.and_then(|d| d.thinking_level));
        self.provider_name = provider_name;
        self.model_name = model_name;

        // Park the outgoing workspace's handles keyed by its canonical
        // root — its actors keep running their turns (events stay tagged
        // with that root) and their command/approval endpoints stay live,
        // so switching back reconnects the still-running session instead
        // of respawning from the last snapshot. `handles.clear()` here
        // used to orphan every actor mid-turn: the stream kept flowing
        // but cancel/steer/approve became unreachable forever.
        let was_active = std::mem::replace(&mut self.workspace_active, true);
        let old_root = std::mem::replace(&mut self.workspace_root, canon);
        let old_handles = std::mem::take(&mut self.handles);
        if was_active && !old_handles.is_empty() {
            self.parked
                .insert(old_root.to_string_lossy().into_owned(), old_handles);
        }
        self.handles = self
            .parked
            .remove(&self.workspace_root.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.store = store;
        self.metas = self.store.list();
        crate::session_store::record_recent_workspace(&self.workspace_root);

        let first = self
            .store
            .last_active()
            .filter(|id| self.metas.iter().any(|m| m.id == *id))
            .or_else(|| self.metas.first().map(|m| m.id));
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

    /// Drop a project from recents and tear down its actors (parked or
    /// active — a running turn is aborted). Session files stay on disk:
    /// reopening the folder restores history. Returns true when the ACTIVE
    /// workspace went away.
    pub fn remove_workspace(&mut self, root: &str) -> bool {
        let canon = std::path::Path::new(root)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(root));
        let key = canon.to_string_lossy().into_owned();
        if let Some(handles) = self.parked.remove(&key) {
            for h in handles.values() {
                h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        crate::session_store::remove_recent_workspace(&canon);
        // The workspace's Serena server (if any) goes with it — a closed
        // project should not keep an indexed child process around.
        crate::tools::serena::shutdown(&canon);

        let active_canon = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());
        let was_active = self.workspace_active && active_canon == canon;
        if was_active {
            for h in self.handles.values() {
                h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.handles.clear();
            self.metas.clear();
            self.active_id = 0;
            self.workspace_active = false;
        }
        was_active
    }

    /// The sidebar's project tree: the active workspace first, then every
    /// recent workspace, each carrying its full persisted conversation list
    /// (newest first).
    ///
    /// The active workspace is read from the *store* rather than
    /// `self.metas` — metas only refresh on structural ops and go stale the
    /// moment an actor persists a turn — and is overlaid with the live
    /// `running`/`active` flags from the handles. Other workspaces are
    /// store-only snapshots: only one workspace has live actors at a time.
    pub fn projects_overview(&self) -> Vec<ProjectOverview> {
        let canon = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());

        let mut out = Vec::new();
        if self.workspace_active {
            out.push(ProjectOverview {
                root: canon.to_string_lossy().into_owned(),
                name: project_name(&canon),
                last_opened: 0,
                current: true,
                sessions: self.current_workspace_rows(),
            });
        }

        for w in self.recent_workspaces() {
            let w_canon = w.path.canonicalize().unwrap_or_else(|_| w.path.clone());
            if self.workspace_active && w_canon == canon {
                // Already first — borrow its recency stamp instead of
                // listing the same project twice.
                out[0].last_opened = w.last_opened;
                continue;
            }
            // Parked handles = this workspace's actors are still live —
            // overlay their real running/preview so the sidebar orb stays
            // on a turn the user switched away from mid-flight.
            let parked_handles = self.parked.get(&w_canon.to_string_lossy().into_owned());
            let sessions = SessionStore::open_existing(&w_canon)
                .map(|store| {
                    store
                        .list()
                        .into_iter()
                        .map(|m| {
                            let live = parked_handles.and_then(|h| h.get(&m.id));
                            ProjectSessionRow {
                                id: m.id,
                                title: m.title,
                                preview: live
                                    .map(|h| h.preview.clone())
                                    .unwrap_or_else(|| m.preview.clone()),
                                updated_at: m.updated_at,
                                active: false,
                                running: live.map(|h| h.running).unwrap_or(false),
                                pinned: m.pinned,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push(ProjectOverview {
                root: w_canon.to_string_lossy().into_owned(),
                name: w.name,
                last_opened: w.last_opened,
                current: false,
                sessions,
            });
        }
        out
    }

    /// Per-session usage records across every known workspace — the settings
    /// 统计 pane's raw material. Deliberately the RAW rows: bucketing by day
    /// needs the reader's timezone, and the frontend already has it, so
    /// aggregating here would only get the dates wrong.
    ///
    /// One entry per session that ever recorded usage (a session with no tokens
    /// yet contributes nothing but noise). `workspace` is the project's display
    /// name, for the per-project breakdown.
    pub fn usage_stats(&self) -> Vec<serde_json::Value> {
        // Recents give nicer names for the projects the user opened through this
        // install; everything else recovers its root from the stored manifest or
        // the history head (see `SessionStore`).
        let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for w in self.recent_workspaces() {
            names.insert(crate::session_store::workspace_key(&w.path), w.name.clone());
        }

        let mut out = Vec::new();
        for store in SessionStore::all_workspaces() {
            // Skip a store with nothing recorded before doing any IO heavy work
            // on its metadata.
            let sessions = store.list();
            if !sessions.iter().any(|m| m.usage.is_some()) {
                continue;
            }
            let root = store.recorded_root().or_else(|| {
                let r = store.recover_root_from_history();
                // Cache it so the next call skips the history read.
                if let Some(p) = &r {
                    store.record_root(p);
                }
                r
            });
            // A temp root is a test artefact, not a project: the kernel's own
            // tests open a store under `tempfile::tempdir()`, which lands here
            // as `/tmp/.tmpXXXX` and would otherwise dominate the breakdown
            // with dozens of one-session, 28-token entries.
            let tmp = std::env::temp_dir();
            if root.as_ref().is_some_and(|r| r.starts_with(&tmp)) {
                continue;
            }
            let key = store.dir_name().unwrap_or_else(|| "unknown".to_string());
            let name = root
                .as_ref()
                .and_then(|r| names.get(&crate::session_store::workspace_key(r)).cloned())
                .or_else(|| root.as_ref().map(|r| project_name(r)))
                .unwrap_or_else(|| format!("项目 {}", &key[..key.len().min(6)]));
            for m in sessions {
                let Some(u) = m.usage else { continue };
                out.push(serde_json::json!({
                    "session": m.id,
                    "title": m.title,
                    "workspace": name,
                    "ts": m.updated_at,
                    "prompt": u.prompt,
                    "completion": u.completion,
                    "cached": u.cached,
                    "context_window": u.context_window,
                    "model": m.model,
                    "provider": m.provider,
                }));
            }
        }
        out
    }

    /// The active workspace's rows, newest first — persisted meta overlaid
    /// with the live preview / running / active flags.
    fn current_workspace_rows(&self) -> Vec<ProjectSessionRow> {
        self.store
            .list()
            .into_iter()
            .map(|m| {
                let live = self.handles.get(&m.id);
                ProjectSessionRow {
                    id: m.id,
                    title: m.title,
                    preview: live
                        .map(|h| h.preview.clone())
                        .unwrap_or_else(|| m.preview.clone()),
                    updated_at: m.updated_at,
                    active: m.id == self.active_id,
                    running: live.map(|h| h.running).unwrap_or(false),
                    pinned: m.pinned,
                }
            })
            .collect()
    }

    /// Spawn an actor for `id` (resume from store if a snapshot exists).
    ///
    /// The session's own persisted `SessionMeta` settings win over the
    /// manager-level workspace default: a session that picked its own model /
    /// permission mode / agent mode / thinking level keeps it across a respawn,
    /// and is never silently re-seeded with whatever another session last set.
    /// `None` fields (a fresh or pre-settings session) fall back to the
    /// workspace default.
    fn spawn_actor(&mut self, id: i64) {
        let meta = self.metas.iter().find(|m| m.id == id).cloned();
        // Per-session overrides — present only after the session itself made a
        // choice (or ran a turn that stamped them). Absent → workspace default.
        let sess_provider = meta
            .as_ref()
            .and_then(|m| m.provider.clone())
            .unwrap_or_else(|| self.provider_name.clone());
        let sess_model = meta
            .as_ref()
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| self.model_name.clone());
        let sess_permission = meta
            .as_ref()
            .and_then(|m| m.permission_mode.clone())
            .unwrap_or_else(|| self.permission_mode.clone());
        let sess_agent_mode = meta
            .as_ref()
            .and_then(|m| m.agent_mode.clone())
            .unwrap_or_else(|| self.agent_mode.clone());
        // `thinking_level` is itself an Option: `and_then` flattens
        // `Option<Option<String>>` so a session that never set one (meta
        // present, field None) still falls back to the workspace default —
        // a `.map` here would unwrap to `Some(None)` and drop the fallback.
        let sess_thinking = meta
            .as_ref()
            .and_then(|m| m.thinking_level.clone())
            .or_else(|| self.active_thinking_level.clone());

        let provider = self
            .provider_cfg
            .providers
            .get(&sess_provider)
            .and_then(|pcfg| ProviderFactory::build(pcfg).ok())
            .unwrap_or_else(|| {
                let (p, _, _) = resolve_provider(&self.provider_cfg);
                p
            });
        let model = sess_model.clone();
        let mentry = self
            .provider_cfg
            .providers
            .get(&sess_provider)
            .and_then(|p| p.find_model(&model));
        // Resolved with `modelOverrides` folded in — the model the user picked
        // is what `model_info` lists, so an override must apply here rather
        // than only to a bare config entry.
        let resolved = mentry.is_some().then(|| {
            self.provider_cfg
                .providers
                .get(&sess_provider)
                .map(|p| p.model_opts(&model))
                .unwrap_or_default()
        });
        let model_params = resolved
            .as_ref()
            .map(ProviderFactory::model_params_from)
            .unwrap_or_default();
        let thinking_level_map = resolved.as_ref().and_then(|d| d.thinking_level_map.clone());
        let model_input = resolved
            .as_ref()
            .map(|d| d.input.clone())
            .unwrap_or_default();
        let context_window = resolved.as_ref().and_then(|d| d.context_window);

        let cfg = SessionConfig {
            workspace_root: self.workspace_root.clone(),
            plugins: self.plugins.clone(),
            provider,
            provider_name: sess_provider.clone(),
            model,
            temperature: 1.0,
            permission_mode: sess_permission.clone(),
            agent_mode: sess_agent_mode.clone(),
            track_dirty: true,
            thinking_level: sess_thinking.clone(),
            thinking_level_map,
            context_window,
            model_input,
            compact_at: Some(self.compact_at),
            model_params: Some(model_params),
            // Restored queue — prompts parked mid-turn persist like history.
            queued_prompts: meta
                .as_ref()
                .map(|m| m.queued_prompts.clone())
                .unwrap_or_default(),
        };

        let (mut actor, channels) = match self.store.load_history(id) {
            Some(hist) => SessionActor::resume(cfg, id, hist),
            // Fresh session — caller-assigned id so store + sidebar agree.
            None => SessionActor::spawn_with_id(cfg, id),
        };

        let cmd_tx = actor.command_sender();
        let decision = actor.decision_writer();
        let ask = actor.ask_channel();
        let permissions = actor.permissions_writer();
        let agent_mode = actor.agent_mode_writer();
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
        // The workspace root is captured AT SPAWN — the actor belongs to
        // this workspace forever, even after the manager switches away
        // and parks its handle. Without the tag, a background session's
        // events would land in the active workspace's id space and
        // corrupt a different session's view.
        // The same sink is kept on the handle for the delivery counters.
        let ui = channels.event_tx.clone();
        {
            let mut rx = channels.event_rx;
            let tx = self.event_tx.clone();
            let root = self.workspace_root.to_string_lossy().into_owned();
            std::thread::Builder::new()
                .name(format!("session-{id}-fwd"))
                .spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
                    rt.block_on(async move {
                        while let Some(ev) = rx.recv().await {
                            if tx.send((root.clone(), id, ev)).is_err() {
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
        self.handles.insert(
            id,
            SessionHandle {
                id,
                cmd_tx,
                decision,
                ask,
                permissions,
                agent_mode,
                thinking_level,
                steer_tx,
                cancel,
                preview,
                running: false,
                ui,
            },
        );
    }

    /// Create a brand-new session (fresh actor, no history) and activate it.
    ///
    /// A new session's settings template is the WORKSPACE default — not the
    /// previous session's per-session choice. `mirror_active_settings` may have
    /// pointed `self.*` at the outgoing session, so we re-resolve the template
    /// from `prefs`+`defaults` here rather than trusting the display mirror.
    pub fn new_session(&mut self) {
        if !self.workspace_active {
            return;
        }
        self.restore_workspace_default_settings();
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
            pinned: false,
            usage: None,
            // Fresh session — no per-session choices yet; every field falls
            // back to the workspace default at spawn.
            model: None,
            provider: None,
            permission_mode: None,
            agent_mode: None,
            thinking_level: None,
            queued_prompts: Vec::new(),
        });
        self.metas = self.store.list();
        self.spawn_actor(id);
        self.active_id = id;
    }

    /// Switch to an existing session — spawns its actor (resumed from the
    /// store) if not already live; the outgoing actor keeps running.
    ///
    /// Activating a session also re-points the manager's *displayed* settings
    /// at that session's own values: `model_info`'s fallback reports
    /// `self.provider_name`/`model_name`/mode/level, so leaving them on the
    /// previous session's choice is exactly the "wrong model in window B" bug.
    /// The session's authoritative values come from its `SessionMeta` (and the
    /// live handle slots); the manager fields are just the active mirror.
    pub fn open_session(&mut self, id: i64) {
        if !self.workspace_active {
            return;
        }
        if !self.handles.contains_key(&id) {
            self.spawn_actor(id);
        }
        self.active_id = id;
        let _ = self.store.set_last_active(id);
        self.mirror_active_settings();
    }

    /// Copy the ACTIVE session's settings into the manager's display fields so
    /// `model_info` and the status bar describe the session now in front — not
    /// the last one that happened to write them. Live handle slots (permission
    /// gate, agent mode, thinking level) are read directly; model/provider come
    /// from the session's `SessionMeta`. Fields the session never set keep the
    /// workspace default.
    fn mirror_active_settings(&mut self) {
        let id = self.active_id;
        if let Some(meta) = self.metas.iter().find(|m| m.id == id).cloned() {
            if let Some(p) = meta.provider.filter(|s| !s.is_empty()) {
                self.provider_name = p;
            }
            if let Some(m) = meta.model.filter(|s| !s.is_empty()) {
                self.model_name = m;
            }
            if let Some(v) = meta.permission_mode.filter(|s| !s.is_empty()) {
                self.permission_mode = v;
            }
            if let Some(v) = meta.agent_mode.filter(|s| !s.is_empty()) {
                self.agent_mode = v;
            }
            if meta.thinking_level.is_some() {
                self.active_thinking_level = meta.thinking_level;
            }
        }
        // Live handle slots override the persisted meta — a mid-session
        // `SetAgentMode`/`SetPermissionMode`/`SetThinkingLevel` writes the slot
        // before the meta round-trip, so the live value is fresher.
        if let Some(h) = self.handles.get(&id) {
            if let Ok(g) = h.permissions.read() {
                self.permission_mode = g.mode().as_str().to_string();
            }
            if let Ok(m) = h.agent_mode.read() {
                self.agent_mode = m.as_str().to_string();
            }
            if let Ok(l) = h.thinking_level.read() {
                self.active_thinking_level = l.clone();
            }
        }
    }

    /// Re-resolve the WORKSPACE default settings (workspace prefs → global
    /// defaults → built-ins) into `self.*`. Called before spawning a NEW
    /// session: a fresh session's template is the workspace default, and
    /// `self.*` may currently be mirroring another session's per-session
    /// choice. Same precedence as `spawn_at`/`switch_workspace`.
    fn restore_workspace_default_settings(&mut self) {
        let prefs = self.store.load_prefs();
        let defaults = crate::session_store::try_load_default_preferences();
        let (_p, default_model, default_pname) = resolve_provider(&self.provider_cfg);
        let (provider_name, model_name) = match (&prefs.provider, &prefs.model) {
            (Some(p), Some(m)) if self.provider_cfg.providers.contains_key(p) => {
                (p.clone(), m.clone())
            }
            _ => (default_pname, default_model),
        };
        self.provider_name = provider_name;
        self.model_name = model_name;
        self.permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| "default".into());
        self.agent_mode = prefs
            .agent_mode
            .or_else(|| defaults.as_ref().map(|d| d.agent_mode.clone()))
            .unwrap_or_else(|| "build".into());
        self.active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.and_then(|d| d.thinking_level));
    }

    /// Delete a session — abort its turn, drop the actor handle (closing
    /// the command channel ends its loop), and remove the store data.
    /// If it was active, switch to the most recent remaining session, or
    /// open a fresh one when none are left.
    pub fn delete_session(&mut self, id: i64) {
        if !self.workspace_active {
            return;
        }
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
        if !self.workspace_active {
            return None;
        }
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
            preview: src.as_ref().map(|m| m.preview.clone()).unwrap_or_default(),
            updated_at: now,
            pinned: src.as_ref().map(|m| m.pinned).unwrap_or(false),
            // The fork inherits the source's meter — its first prompt
            // re-samples the same history, so the number is a fair stand-in
            // until that turn's real `Usage` lands.
            usage: src.as_ref().and_then(|m| m.usage),
            model: src.as_ref().and_then(|m| m.model.clone()),
            provider: src.as_ref().and_then(|m| m.provider.clone()),
            // The fork inherits the source's composer settings too — a copy
            // should run under the same model/mode/level its source was using.
            permission_mode: src.as_ref().and_then(|m| m.permission_mode.clone()),
            agent_mode: src.as_ref().and_then(|m| m.agent_mode.clone()),
            thinking_level: src.as_ref().and_then(|m| m.thinking_level.clone()),
            // The fork copies history, not pending intent — parked prompts
            // stay with the source session.
            queued_prompts: Vec::new(),
        });
        self.metas = self.store.list();
        self.spawn_actor(new_id);
        self.active_id = new_id;
        let _ = self.store.set_last_active(new_id);
        Some(new_id)
    }

    /// Discover + register every MCP server for `root`. `None` when there is
    /// nothing to load, so an empty plugin dir costs no runtime.
    fn load_plugins(root: &std::path::Path) -> Option<Arc<agent_plugin::PluginManager>> {
        if agent_plugin::discover(root).is_empty() {
            return None;
        }
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let mgr = rt.block_on(agent_plugin::load_all(root));
        tracing::info!(tools = mgr.exported_tools().len(), "MCP plugins loaded");
        Some(Arc::new(mgr))
    }

    /// Re-read the plugin dirs and swap the live router.
    ///
    /// Plugins are otherwise loaded once at boot, so a server added in settings
    /// stayed invisible to every running (and future) session until the app was
    /// restarted. Sessions hold the same handle the engine reads per request,
    /// so the swap takes effect on the next turn — nothing is restarted.
    pub fn reload_plugins(&self) -> Vec<serde_json::Value> {
        match &self.plugins {
            Some(handle) => {
                let loaded = Self::load_plugins(&self.workspace_root);
                let summary = loaded.as_ref().map(|m| m.status()).unwrap_or_default();
                *handle.write().unwrap() = loaded;
                summary
            }
            None => Vec::new(),
        }
    }

    /// Connect one MCP server and report what it actually is: the server's own
    /// name/version plus its tool names.
    ///
    /// The settings card used to show the manifest's `version` — written as
    /// "1.0.0" for anything added through the UI — and a count read from the
    /// manifest's declared capabilities, which is empty for a server reached
    /// over HTTP, so a working two-tool server read as "0 工具". Nothing ever
    /// connected a plugin, so there was no live number to show. Failures come
    /// back as data (`ok: false`) so the card can print the reason.
    pub async fn mcp_probe(workspace_root: &std::path::Path, id: &str) -> serde_json::Value {
        let Some(mpath) = agent_plugin::discover(workspace_root)
            .into_iter()
            .find(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    == Some(id)
            })
        else {
            return serde_json::json!({ "ok": false, "error": format!("找不到插件 {id}") });
        };
        let Ok(text) = std::fs::read_to_string(&mpath) else {
            return serde_json::json!({ "ok": false, "error": "manifest 读取失败" });
        };
        let Ok(manifest) = serde_json::from_str::<agent_plugin::PluginManifest>(&text) else {
            return serde_json::json!({ "ok": false, "error": "manifest 解析失败" });
        };
        match agent_plugin::McpClient::start(&manifest).await {
            Ok(client) => {
                let (name, version) = client.server_info();
                serde_json::json!({
                    "ok": true,
                    "serverName": name,
                    "serverVersion": version,
                    "tools": client.tool_names().await,
                })
            }
            Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
        }
    }

    /// Delete a discovered plugin's directory (settings UI). The path comes
    /// from discovery, so an id can never point outside the plugins dirs.
    pub fn remove_plugin(workspace_root: &std::path::Path, id: &str) -> Result<(), String> {
        let mpath = agent_plugin::discover(workspace_root)
            .into_iter()
            .find(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    == Some(id)
            })
            .ok_or_else(|| format!("找不到插件 {id}"))?;
        let dir = mpath.parent().ok_or("插件目录异常")?;
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
    }

    /// Discovered MCP/plugin manifests for the settings UI — reads
    /// `manifest.json` files under `~/.config/husk/plugins/` and
    /// `<repo>/.agent/plugins/`. Listing only: this never spawns a server.
    pub fn plugin_overview(&self) -> Vec<serde_json::Value> {
        // Live state of the connections loaded at boot (`spawn_at`) — memory
        // only. Reconnecting here would open a fresh MCP session per plugin on
        // every visit to the settings pane, which is what tripped rate limits.
        let live: std::collections::HashMap<String, serde_json::Value> = self
            .plugins
            .as_ref()
            .and_then(|h| h.read().ok().and_then(|g| g.clone()))
            .map(|m| {
                m.status()
                    .into_iter()
                    .map(|v| {
                        let id = v["id"].as_str().unwrap_or_default().to_string();
                        (id, v)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut out = Vec::new();
        for mpath in agent_plugin::discover(&self.workspace_root) {
            let Ok(text) = std::fs::read_to_string(&mpath) else {
                continue;
            };
            let Ok(m) = serde_json::from_str::<agent_plugin::PluginManifest>(&text) else {
                continue;
            };
            let id = m.id.clone();
            let st = live.get(&id);
            let field = |k: &str| st.map(|v| v[k].clone()).unwrap_or(serde_json::Value::Null);
            out.push(serde_json::json!({
                "id": id,
                "name": m.name,
                "version": m.version,
                "kind": match m.kind {
                    agent_plugin::PluginKind::Mcp => "mcp",
                    agent_plugin::PluginKind::Wasm => "wasm",
                },
                "entry": m.entry,
                // Live tool names from the loaded handshake (the manifest's
                // own `capabilities.tools` is empty for HTTP servers).
                "tools": field("tools"),
                "commands": m.capabilities.commands.len(),
                "sandboxed": m.sandboxed,
                "dir": m.dir.to_string_lossy(),
                "connected": field("connected"),
                "serverVersion": field("serverVersion"),
                "error": field("error"),
            }));
        }
        out
    }

    /// The two user-instruction files the settings "指令" pane edits —
    /// `~/.config/husk/AGENTS.md` (user-wide) and `<workspace>/AGENTS.md`
    /// (project). Both append to every new session's system prompt.
    pub fn instructions(&self) -> serde_json::Value {
        let global = dirs::config_dir().map(|d| d.join("husk/AGENTS.md"));
        let ws = self.workspace_root.join("AGENTS.md");
        let read = |p: &Option<std::path::PathBuf>| {
            p.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default()
        };
        serde_json::json!({
            "global": {
                "path": global.as_ref().map(|p| p.to_string_lossy().to_string()),
                "content": read(&global),
            },
            "workspace": {
                "path": ws.to_string_lossy().to_string(),
                "content": std::fs::read_to_string(&ws).unwrap_or_default(),
            },
        })
    }

    /// Write one instructions file (`global` | `workspace`) — creates the
    /// config dir on first save.
    pub fn set_instructions(&self, scope: &str, content: &str) -> Result<(), String> {
        let path = match scope {
            "global" => dirs::config_dir()
                .map(|d| d.join("husk/AGENTS.md"))
                .ok_or("no config dir")?,
            "workspace" => self.workspace_root.join("AGENTS.md"),
            _ => return Err(format!("unknown instructions scope `{scope}`")),
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }

    /// Delivery counters for every live session (active workspace + parked
    /// workspaces) — the shell's status bar surfaces `dropped`/`coalesced`
    /// so a lagging consumer stops being invisible.
    pub fn ui_stats(&self) -> UiStatsSnapshot {
        self.parked
            .values()
            .flat_map(|sessions| sessions.values())
            .map(|h| h.ui.stats())
            .chain(self.handles.values().map(|h| h.ui.stats()))
            .fold(UiStatsSnapshot::default(), |mut acc, s| {
                acc.add(&s);
                acc
            })
    }

    /// The active session's handles (for UI commands).
    pub fn active(&self) -> Option<&SessionHandle> {
        self.handles.get(&self.active_id)
    }

    /// Load a session's persisted history (for rebuilding a closed view).
    pub fn store_history(&self, id: i64) -> Option<Vec<agent_llm::types::ChatMessage>> {
        self.store.load_history(id)
    }

    /// Paged history — `(slice, total_len, turn_total, turn_offset)`.
    /// `before` is an exclusive end index into the full message list;
    /// `None` takes the tail page. `turn_offset` is the ordinal (among
    /// non-hidden user messages) of the first turn that lands *inside*
    /// this slice, so the rail can name the unloaded turns below it
    /// exactly instead of estimating from an average turn length.
    /// The webview mounts history in pages instead of all at once: a
    /// 65k-node session made every frame pathological on WebKitGTK, so
    /// `open` returns only the newest page and scroll-up fetches older
    /// slices via `history_page`.
    pub fn store_history_page(
        &self,
        id: i64,
        before: Option<usize>,
        count: usize,
    ) -> (Vec<agent_llm::types::ChatMessage>, usize, usize, usize) {
        if !self.workspace_active {
            return (Vec::new(), 0, 0, 0);
        }
        let hist = self.store.load_history(id).unwrap_or_default();
        let total = hist.len();
        // Visible turns ≈ non-hidden user messages — the rail sizes its
        // overview track against this so unloaded history still occupies
        // its true share of the strip.
        fn is_turn(m: &agent_llm::types::ChatMessage) -> bool {
            m.role == agent_llm::types::Role::User
                && m.notice != Some(agent_llm::types::NoticeKind::Hidden)
        }
        let turn_total = hist.iter().filter(|m| is_turn(m)).count();
        let end = before.map(|b| b.min(total)).unwrap_or(total);
        let start = end.saturating_sub(count);
        let turn_offset = hist[..start].iter().filter(|m| is_turn(m)).count();
        (hist[start..end].to_vec(), total, turn_total, turn_offset)
    }

    /// A session's persisted last-turn usage — seeds the header meter when
    /// the webview rebuilds a closed view (before any new `Usage` event).
    /// Reads the store fresh: `self.metas` only refreshes on structural ops
    /// and goes stale the moment an actor persists a turn.
    pub fn store_usage(&self, id: i64) -> Option<crate::session_store::SessionUsage> {
        if !self.workspace_active {
            return None;
        }
        self.store
            .list()
            .into_iter()
            .find(|m| m.id == id)
            .and_then(|m| m.usage)
    }

    /// A session's persisted queued prompts — seeds the composer's parked
    /// list when the webview rebuilds a closed view. Reads the store fresh
    /// for the same reason `store_usage` does.
    pub fn store_queued(&self, id: i64) -> Vec<String> {
        if !self.workspace_active {
            return Vec::new();
        }
        self.store
            .list()
            .into_iter()
            .find(|m| m.id == id)
            .map(|m| m.queued_prompts)
            .unwrap_or_default()
    }

    /// Sidebar rows — persisted metas overlaid with live running/preview.
    /// Tuple: (id, title, preview, active, running, pinned).
    pub fn sidebar_rows(&self) -> Vec<(i64, String, String, bool, bool, bool)> {
        if !self.workspace_active {
            return Vec::new();
        }
        self.metas
            .iter()
            .map(|m| {
                let live = self.handles.get(&m.id);
                let preview = live
                    .map(|h| h.preview.clone())
                    .unwrap_or_else(|| m.preview.clone());
                let running = live.map(|h| h.running).unwrap_or(false);
                (
                    m.id,
                    m.title.clone(),
                    preview,
                    m.id == self.active_id,
                    running,
                    m.pinned,
                )
            })
            .collect()
    }

    /// Pin/unpin a session in the sidebar. Returns the new flag value —
    /// `Err` only when the id isn't in the index.
    pub fn pin_session(&mut self, id: i64, pinned: bool) -> std::io::Result<bool> {
        if !self.workspace_active {
            return Ok(false);
        }
        let ok = self.store.set_pinned(id, pinned)?;
        if ok {
            self.metas = self.store.list();
        }
        Ok(ok)
    }

    /// mtime of the config file — the gate for a disk reload.
    fn config_stamp_of(path: Option<&std::path::Path>) -> Option<std::time::SystemTime> {
        path.map(|p| p.to_path_buf())
            .or_else(AppConfig::default_path)
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
    }

    fn config_stamp_now() -> Option<std::time::SystemTime> {
        Self::config_stamp_of(None)
    }

    /// Re-read `config.toml` when it changed on disk and hot-apply it to every
    /// live actor. Returns true when a reload happened.
    ///
    /// An actor keeps the provider instance and model parameters it was spawned
    /// with, so an edit made in the settings used to surface only on restart.
    pub fn reload_model_config(&mut self) -> bool {
        self.reload_model_config_from(None)
    }

    /// `path` overrides the config file — tests drive the whole load path
    /// against a temp file instead of the user's config.
    pub fn reload_model_config_from(&mut self, path: Option<&std::path::Path>) -> bool {
        let stamp = Self::config_stamp_of(path);
        if stamp.is_none() || stamp == self.config_stamp {
            return false;
        }
        let Ok(cfg) = AppConfig::load(path) else {
            return false;
        };
        self.config_stamp = stamp;
        let (notified, live) = self.apply_loaded_config(cfg);
        // A full command queue drops the reload; forget the stamp so the next
        // check retries instead of leaving that actor on the old parameters.
        if notified < live {
            self.config_stamp = None;
        }
        true
    }

    /// Rules for a freshly loaded config, split from the disk I/O so they are
    /// testable without touching the user's config: refresh the cached copy,
    /// re-resolve the active model when the edit removed it, and hand the pair
    /// to every live actor. Returns `(notified, live)`.
    pub fn apply_loaded_config(&mut self, cfg: AppConfig) -> (usize, usize) {
        self.provider_cfg = cfg;
        if !self.active_model_is_configured() {
            let (_, model, provider) = resolve_provider(&self.provider_cfg);
            self.provider_name = provider;
            self.model_name = model;
        }
        self.notify_live_actors()
    }

    /// Whether the active model is still in the loaded config. A model deleted
    /// in the settings would otherwise leave every turn pointed at a dead id.
    fn active_model_is_configured(&self) -> bool {
        let Some(pcfg) = self.provider_cfg.providers.get(&self.provider_name) else {
            return false;
        };
        pcfg.find_model(&self.model_name).is_some()
            // A provider that lists no models accepts any id.
            || (pcfg.models.is_empty() && pcfg.default_model.is_none())
            || pcfg.default_model.as_deref() == Some(self.model_name.as_str())
    }

    /// Push a reload into every live actor — the active session's and the parked
    /// workspaces' (they keep running, and keep their endpoints, while switched
    /// away). Returns `(notified, live)`.
    fn notify_live_actors(&self) -> (usize, usize) {
        let cmd = agent_ipc::UiCommand::ReloadModel {
            provider: self.provider_name.clone(),
            model: self.model_name.clone(),
        };
        let mut notified = 0;
        let mut live = 0;
        for h in self
            .handles
            .values()
            .chain(self.parked.values().flat_map(|m| m.values()))
        {
            live += 1;
            if h.cmd_tx.try_send(cmd.clone()).is_ok() {
                notified += 1;
            }
        }
        (notified, live)
    }

    /// Model info (active provider, active model, active thinking level, configured models list, config path).
    pub fn model_info(&mut self) -> SessionModelInfo {
        // Pick up an edit in `config.toml` (and pass it on to live actors) so no
        // change needs a restart.
        self.reload_model_config();

        let mut models = Vec::new();
        for (pname, pcfg) in &self.provider_cfg.providers {
            if !pcfg.models.is_empty() {
                for m in &pcfg.models {
                    let d = m.detailed();
                    let reasoning = d.and_then(|x| x.reasoning).unwrap_or(false)
                        || d.and_then(|x| x.thinking_level_map.as_ref()).is_some();
                    let thinking_level_map = d.and_then(|x| x.thinking_level_map.clone());
                    // Levels come ONLY from the model's thinking_level_map
                    // — a model with `reasoning: true` but no map exposes no
                    // selectable levels, and the picker stays hidden.
                    let mut available_levels = Vec::new();
                    if let Some(map) = &thinking_level_map {
                        for lvl in &["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
                            if *lvl == "off" {
                                available_levels.push("off".to_string());
                            } else if let Some(Some(_)) = map.get(*lvl) {
                                available_levels.push(lvl.to_string());
                            }
                        }
                    }

                    models.push(ModelDetails {
                        provider: pname.clone(),
                        model: m.id().to_string(),
                        name: Some(m.name().to_string()),
                        reasoning,
                        thinking_level_map,
                        available_levels,
                        context_window: d.and_then(|x| x.context_window),
                        max_tokens: d
                            .and_then(|x| x.max_tokens)
                            .map(|v| v.min(u32::MAX as u64) as u32),
                        cost: d.and_then(|x| x.cost.clone()),
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
                    context_window: None,
                    max_tokens: None,
                    cost: None,
                });
            } else {
                models.push(ModelDetails {
                    provider: pname.clone(),
                    model: self.model_name.clone(),
                    name: Some(self.model_name.clone()),
                    reasoning: false,
                    thinking_level_map: None,
                    available_levels: Vec::new(),
                    context_window: None,
                    max_tokens: None,
                    cost: None,
                });
            }
        }

        if !models
            .iter()
            .any(|m| m.provider == self.provider_name && m.model == self.model_name)
        {
            models.insert(
                0,
                ModelDetails {
                    provider: self.provider_name.clone(),
                    model: self.model_name.clone(),
                    name: Some(self.model_name.clone()),
                    reasoning: false,
                    thinking_level_map: None,
                    available_levels: Vec::new(),
                    context_window: None,
                    max_tokens: None,
                    cost: None,
                },
            );
        }

        if self.active_thinking_level.is_none() {
            if let Some(m) = models
                .iter()
                .find(|m| m.provider == self.provider_name && m.model == self.model_name)
            {
                if m.reasoning {
                    if m.available_levels.contains(&"medium".to_string()) {
                        self.active_thinking_level = Some("medium".into());
                    } else if let Some(first_non_off) =
                        m.available_levels.iter().find(|l| *l != "off")
                    {
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
        let (live_mode, live_level, live_agent_mode) = self
            .active()
            .map(|h| {
                let m = h
                    .permissions
                    .read()
                    .ok()
                    .map(|g| g.mode().as_str().to_string());
                let t = h.thinking_level.read().ok().and_then(|l| l.clone());
                let a = h
                    .agent_mode
                    .read()
                    .ok()
                    .map(|mode| mode.as_str().to_string());
                (m, t, a)
            })
            .unwrap_or((None, None, None));

        let path = AppConfig::default_path().map(|p| p.to_string_lossy().to_string());
        SessionModelInfo {
            active_provider: self.provider_name.clone(),
            active_model: self.model_name.clone(),
            active_thinking_level: live_level.or_else(|| self.active_thinking_level.clone()),
            active_permission_mode: live_mode.or_else(|| Some(self.permission_mode.clone())),
            // Same rule as the gate/level above. Reporting the manager-level
            // default here made plan mode look workspace-wide: a plan ran in
            // one session, and the "计划已就绪 / 批准并执行" strip then
            // appeared in every other session that happened to end on an
            // assistant block.
            active_agent_mode: live_agent_mode.or_else(|| Some(self.agent_mode.clone())),
            config_path: path,
            models,
        }
    }

    /// The stored default preferences — what NEW sessions spawn with.
    /// The settings popup reads/writes these; already-running sessions keep
    /// whatever they were spawned (or later switched) with.
    pub fn default_prefs(&self) -> (String, Option<String>, String, f32) {
        (
            self.permission_mode.clone(),
            self.active_thinking_level.clone(),
            self.agent_mode.clone(),
            self.compact_at,
        )
    }

    /// Update the stored default preferences — applies to sessions spawned
    /// AFTER this call; live sessions are untouched (no gate write, no
    /// UiCommand, no SystemMessage in their stream).
    pub fn set_default_prefs(
        &mut self,
        permission_mode: Option<String>,
        thinking_level: Option<String>,
        agent_mode: Option<String>,
        compact_at: Option<f32>,
    ) {
        if let Some(m) = permission_mode {
            self.permission_mode = m;
        }
        if let Some(l) = thinking_level {
            self.active_thinking_level = if l.is_empty() { None } else { Some(l) };
        }
        if let Some(m) = agent_mode {
            self.agent_mode = m;
        }
        if let Some(f) = compact_at {
            self.compact_at = f.clamp(0.5, 0.95);
        }
        self.persist_prefs();
        // Also persist the global defaults so fresh workspaces inherit them.
        // Sandbox overrides live in the process-wide slot — read it back so
        // the file always mirrors the effective values.
        let lim = crate::sandbox_prefs::current();
        let _ = crate::session_store::save_default_preferences(
            &crate::session_store::DefaultPreferences {
                permission_mode: self.permission_mode.clone(),
                thinking_level: self.active_thinking_level.clone(),
                agent_mode: self.agent_mode.clone(),
                compact_at: self.compact_at,
                sandbox_network: Some(lim.network_label().into()),
                sandbox_max_memory_mb: lim.max_memory_mb,
                sandbox_max_processes: lim.max_processes,
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
            agent_mode: Some(self.agent_mode.clone()),
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

    /// Update active agent mode in manager metadata and persist to
    /// workspace prefs — the session's live swap happens through the
    /// `SetAgentMode` command the caller also pumps in.
    pub fn set_agent_mode(&mut self, mode: String) {
        self.agent_mode = mode;
        self.persist_prefs();
    }
}

/// Resolve the active provider from `config.toml`.
fn resolve_provider(cfg: &AppConfig) -> (Arc<dyn agent_llm::LlmProvider>, String, String) {
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
        Arc::new(agent_llm::provider::UnconfiguredProvider),
        "default".into(),
        "no provider configured".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config edit must reach the actors that are already running — an actor
    /// keeps the provider instance and model parameters it was spawned with, and
    /// a model that the edit removed must not stay the session's target.
    #[test]
    fn config_reload_reaches_live_actors_and_re_resolves_a_removed_model() {
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()));

        // Stand-in for a live session actor — only its command queue matters.
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(4);
        let (ui, _ui_rx) = crate::channels::UiSink::channel();
        mgr.handles.insert(
            i64::MAX,
            SessionHandle {
                id: i64::MAX,
                cmd_tx,
                decision: Arc::new(Mutex::new(None)),
                ask: Arc::new(crate::tools::registry::AskChannel::new(None)),
                permissions: Arc::new(std::sync::RwLock::new(
                    crate::permissions::PermissionGate::from_mode_str("default"),
                )),
                agent_mode: Arc::new(std::sync::RwLock::new(crate::mode::AgentMode::Build)),
                thinking_level: Arc::new(std::sync::RwLock::new(None)),
                steer_tx: tokio::sync::mpsc::channel(1).0,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                preview: String::new(),
                running: false,
                ui,
            },
        );

        // The edit keeps one provider but renames its only model.
        let cfg: AppConfig = serde_json::from_value(serde_json::json!({
            "active_provider": "devin",
            "providers": {
                "devin": {
                    "kind": "openai_compat",
                    "base_url": "https://example.invalid",
                    "default_model": "devin/swe-3",
                }
            }
        }))
        .unwrap();

        let (notified, live) = mgr.apply_loaded_config(cfg);
        assert!(live >= 1, "the inserted handle is live");
        assert_eq!(notified, live, "every live actor is notified");
        assert_eq!(mgr.provider_name, "devin");
        assert_eq!(mgr.model_name, "devin/swe-3");

        let mut sent = Vec::new();
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let agent_ipc::UiCommand::ReloadModel { provider, model } = cmd {
                sent.push((provider, model));
            }
        }
        assert_eq!(sent, vec![("devin".to_string(), "devin/swe-3".to_string())]);
    }

    /// One `config.toml` writer for the reload tests.
    fn write_config(path: &std::path::Path, provider: &str, model: &str) {
        std::fs::write(
            path,
            format!(
                "active_provider = \"{provider}\"\n\
                 [providers.{provider}]\n\
                 kind = \"openai_compat\"\n\
                 base_url = \"https://example.invalid\"\n\
                 default_model = \"{model}\"\n"
            ),
        )
        .unwrap();
    }

    /// The whole load path: a config file that changed on disk is read once and
    /// applied, and a model the edit removed re-resolves instead of leaving the
    /// session pointed at a dead id.
    #[test]
    fn reload_model_config_reads_the_file_once_and_re_resolves_a_removed_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_config(&path, "devin", "devin/swe-2");
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()));

        assert!(
            mgr.reload_model_config_from(Some(&path)),
            "changed file loads"
        );
        assert_eq!(
            (mgr.provider_name.as_str(), mgr.model_name.as_str()),
            ("devin", "devin/swe-2")
        );
        assert!(
            !mgr.reload_model_config_from(Some(&path)),
            "unchanged file is not re-read"
        );

        write_config(&path, "other", "other/big");
        // Recreating the file can land on the same mtime — move it forward.
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(1))
            .unwrap();
        drop(f);

        assert!(
            mgr.reload_model_config_from(Some(&path)),
            "the rewrite is picked up"
        );
        assert_eq!(
            (mgr.provider_name.as_str(), mgr.model_name.as_str()),
            ("other", "other/big")
        );
    }

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
        let (mut mgr2, _rx) = SessionManager::spawn_at(Some(path));
        assert_eq!(mgr2.provider_name, "devin");
        assert_eq!(mgr2.model_name, "devin/swe-2");
        assert_eq!(mgr2.active_thinking_level.as_deref(), Some("high"));
        assert_eq!(mgr2.permission_mode, "acceptEdits");
        let info = mgr2.model_info();
        assert_eq!(info.active_permission_mode.as_deref(), Some("acceptEdits"));
        assert_eq!(info.active_thinking_level.as_deref(), Some("high"));
    }

    /// `model_info` answers for the ACTIVE session, not for the workspace. The
    /// composer's plan hand-off keys off `active_agent_mode`, and reporting the
    /// manager's workspace-wide default lit the "计划已就绪 / 批准并执行" strip
    /// up in every session that happened to end on an assistant block.
    #[test]
    fn model_info_reports_the_active_sessions_agent_mode() {
        use crate::mode::AgentMode;

        /// A stand-in actor — only its agent-mode slot is read here.
        fn fake_handle(
            id: i64,
            mode: AgentMode,
        ) -> (
            SessionHandle,
            tokio::sync::mpsc::Receiver<agent_ipc::UiCommand>,
        ) {
            let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(4);
            let (ui, _ui_rx) = crate::channels::UiSink::channel();
            (
                SessionHandle {
                    id,
                    cmd_tx,
                    decision: Arc::new(Mutex::new(None)),
                    ask: Arc::new(crate::tools::registry::AskChannel::new(None)),
                    permissions: Arc::new(std::sync::RwLock::new(
                        crate::permissions::PermissionGate::from_mode_str("default"),
                    )),
                    agent_mode: Arc::new(std::sync::RwLock::new(mode)),
                    thinking_level: Arc::new(std::sync::RwLock::new(None)),
                    steer_tx: tokio::sync::mpsc::channel(1).0,
                    cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    preview: String::new(),
                    running: false,
                    ui,
                },
                cmd_rx,
            )
        }

        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()));
        mgr.workspace_active = true;

        // Receivers stay alive: a dropped one closes the queue the config
        // reload pushes `ReloadModel` into.
        let mut _live = Vec::new();
        for (id, mode) in [(1, AgentMode::Plan), (2, AgentMode::Build)] {
            let (handle, rx) = fake_handle(id, mode);
            mgr.handles.insert(id, handle);
            _live.push(rx);
        }

        // A stored default for NEW sessions — must not leak into the report.
        mgr.agent_mode = "plan".into();

        mgr.active_id = 2;
        assert_eq!(mgr.model_info().active_agent_mode.as_deref(), Some("build"));
        mgr.active_id = 1;
        assert_eq!(mgr.model_info().active_agent_mode.as_deref(), Some("plan"));
    }

    /// The core regression: switching sessions must re-point the manager's
    /// displayed settings at THAT session's own persisted choice — the
    /// "window B shows window A's model" bug. Two sessions with different
    /// stored models; activating each mirrors its own meta.
    #[test]
    fn switching_sessions_mirrors_each_sessions_own_model() {
        use crate::session_store::SessionMeta;
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()));
        mgr.workspace_active = true;

        // Two sessions, each persisted a different model/provider. Handles are
        // fake — `open_session` sees them as already-live and skips respawn.
        for (id, provider, model) in [(1i64, "openai", "gpt-5"), (2i64, "deepseek", "ds-v3")] {
            let meta = SessionMeta {
                id,
                title: format!("s{id}"),
                preview: String::new(),
                updated_at: 0,
                pinned: false,
                usage: None,
                model: Some(model.into()),
                provider: Some(provider.into()),
                permission_mode: Some("default".into()),
                agent_mode: Some("build".into()),
                thinking_level: None,
                queued_prompts: Vec::new(),
            };
            mgr.store.upsert_meta(meta).unwrap();
            let (cmd_tx, _rx) = tokio::sync::mpsc::channel(4);
            let (ui, _ui_rx) = crate::channels::UiSink::channel();
            mgr.handles.insert(
                id,
                SessionHandle {
                    id,
                    cmd_tx,
                    decision: Arc::new(Mutex::new(None)),
                    ask: Arc::new(crate::tools::registry::AskChannel::new(None)),
                    permissions: Arc::new(std::sync::RwLock::new(
                        crate::permissions::PermissionGate::from_mode_str("default"),
                    )),
                    agent_mode: Arc::new(std::sync::RwLock::new(crate::mode::AgentMode::Build)),
                    thinking_level: Arc::new(std::sync::RwLock::new(None)),
                    steer_tx: tokio::sync::mpsc::channel(1).0,
                    cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    preview: String::new(),
                    running: false,
                    ui,
                },
            );
        }
        mgr.metas = mgr.store.list();

        // Activate session 1 — its model becomes the displayed one.
        mgr.open_session(1);
        assert_eq!(mgr.provider_name, "openai");
        assert_eq!(mgr.model_name, "gpt-5");

        // Switch to session 2 — the display must follow IT, not stay on 1's.
        mgr.open_session(2);
        assert_eq!(mgr.provider_name, "deepseek");
        assert_eq!(mgr.model_name, "ds-v3");
        let info = mgr.model_info();
        assert_eq!(info.active_provider, "deepseek");
        assert_eq!(info.active_model, "ds-v3");
    }

    /// A session that never picked its own settings falls back to the
    /// workspace default on activation — per-session fields are `None` until
    /// the session itself chooses.
    #[test]
    fn a_session_without_settings_falls_back_to_the_workspace_default() {
        use crate::session_store::SessionMeta;
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()));
        mgr.workspace_active = true;
        // Workspace default the fresh session should inherit.
        mgr.provider_name = "fallback-p".into();
        mgr.model_name = "fallback-m".into();

        // Session meta with NO per-session choices — every field falls back.
        let meta = SessionMeta {
            id: 7,
            title: "untouched".into(),
            preview: String::new(),
            updated_at: 0,
            pinned: false,
            usage: None,
            model: None,
            provider: None,
            permission_mode: None,
            agent_mode: None,
            thinking_level: None,
            queued_prompts: Vec::new(),
        };
        mgr.store.upsert_meta(meta).unwrap();
        mgr.metas = mgr.store.list();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel(4);
        let (ui, _ui_rx) = crate::channels::UiSink::channel();
        mgr.handles.insert(
            7,
            SessionHandle {
                id: 7,
                cmd_tx,
                decision: Arc::new(Mutex::new(None)),
                ask: Arc::new(crate::tools::registry::AskChannel::new(None)),
                permissions: Arc::new(std::sync::RwLock::new(
                    crate::permissions::PermissionGate::from_mode_str("auto"),
                )),
                agent_mode: Arc::new(std::sync::RwLock::new(crate::mode::AgentMode::Plan)),
                thinking_level: Arc::new(std::sync::RwLock::new(None)),
                steer_tx: tokio::sync::mpsc::channel(1).0,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                preview: String::new(),
                running: false,
                ui,
            },
        );

        mgr.open_session(7);
        // Model/provider fall back to the workspace default (meta unset); the
        // live handle's mode/level slots still override where they exist.
        assert_eq!(mgr.provider_name, "fallback-p");
        assert_eq!(mgr.model_name, "fallback-m");
        assert_eq!(mgr.permission_mode, "auto");
        assert_eq!(mgr.agent_mode, "plan");
    }
}
