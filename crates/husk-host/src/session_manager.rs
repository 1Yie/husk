//! `session_manager` — multi-session kernel wiring, frontend-agnostic.
//!
//! Each session owns a `SessionActor` on its own `kernel-rt` thread, so a
//! background turn keeps running when the user switches away. Every actor's
//! `UiEvent` stream is forwarded into one channel tagged with its `SessionId`.
//! `SessionStore` snapshots history at turn boundaries; `open_session` resumes
//! from the last snapshot.

use std::collections::HashMap;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use crate::session::{SessionActor, SessionConfig};
use agent_ipc::UiEvent;
use agent_kernel::channels::{UiSink, UiStatsSnapshot};
use agent_llm::{AppConfig, ProviderFactory};
use agent_paths::AppPaths;
use agent_store::{SessionMeta, SessionStore};
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
    /// Pinned within the workspace — leads the project's list.
    pub pinned: bool,
    /// Pinned globally — leads the cross-project 会话 list.
    #[serde(default)]
    pub pinned_global: bool,
    /// Archived — the sidebar shows it only under 已归档.
    #[serde(default)]
    pub archived: bool,
}

/// The store a sidebar session op addresses — see `edit_session_at`.
enum SessionTarget {
    Active,
    Other(SessionStore),
    Missing,
}

/// One project (workspace) with its full conversation list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOverview {
    pub root: String,
    pub name: String,
    pub last_opened: u64,
    /// This is the kernel's active workspace.
    pub current: bool,
    /// The built-in office pseudo-workspace (`~/.local/share/husk/office`)
    /// — the sidebar renders it as a fixed「办公」section instead of a
    /// regular project, and its sessions stay readable from every mode.
    #[serde(default)]
    pub office: bool,
    /// Every persisted conversation, newest first.
    pub sessions: Vec<ProjectSessionRow>,
}

/// Display name for a workspace — its final path component, or the full
/// path when there is none (filesystem root). The office pseudo-workspace
/// gets a fixed「办公」label instead of the directory name.
fn project_name(root: &std::path::Path) -> String {
    if crate::office::is_office_root(root) {
        return "办公".into();
    }
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

/// What the project tree needs from the manager, copied out under its lock:
/// which store backs each project and the live overlay of every actor. Reading
/// the stores (`build`) parses every session index on disk — half a second on
/// a long-lived install — so it runs after the lock is released. Holding the
/// manager for that long stalled the event forwarder behind it, and every
/// event of every running session arrived late.
pub struct ProjectsSnapshot {
    projects: Vec<SnapshotProject>,
}

struct SnapshotProject {
    root: String,
    name: String,
    last_opened: u64,
    current: bool,
    office: bool,
    store: StoreRef,
    /// `(preview, running)` per live actor, keyed by session id.
    live: HashMap<i64, (String, bool)>,
    /// The active session of the current project — `0` everywhere else.
    active_id: i64,
}

/// Which store backs a project. Opening one reads its workspace row from disk,
/// so the snapshot keeps the path and `build` opens it.
enum StoreRef {
    /// The active workspace's store, already open on the manager.
    Open(Arc<SessionStore>),
    /// Any other project — opened only if it has session data.
    Existing(std::path::PathBuf),
}

impl StoreRef {
    fn open(self) -> Option<Arc<SessionStore>> {
        match self {
            Self::Open(store) => Some(store),
            Self::Existing(root) => SessionStore::open_existing(&root).map(Arc::new),
        }
    }
}

impl ProjectsSnapshot {
    /// Read every project's session list from disk. No manager state is
    /// touched, so this must run without the manager lock.
    pub fn build(self) -> Vec<ProjectOverview> {
        self.projects
            .into_iter()
            .map(|p| {
                let sessions = p
                    .store
                    .open()
                    .map(|store| {
                        store
                            .list()
                            .into_iter()
                            .map(|m| {
                                let live = p.live.get(&m.id);
                                ProjectSessionRow {
                                    id: m.id,
                                    title: m.title,
                                    preview: live
                                        .map(|(preview, _)| preview.clone())
                                        .unwrap_or(m.preview),
                                    updated_at: m.updated_at,
                                    active: p.current && m.id == p.active_id,
                                    running: live.is_some_and(|(_, running)| *running),
                                    pinned: m.pinned,
                                    pinned_global: m.pinned_global,
                                    archived: m.archived,
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                ProjectOverview {
                    root: p.root,
                    name: p.name,
                    last_opened: p.last_opened,
                    current: p.current,
                    office: p.office,
                    sessions,
                }
            })
            .collect()
    }
}

fn live_overlay(handles: Option<&HashMap<i64, SessionHandle>>) -> HashMap<i64, (String, bool)> {
    handles
        .into_iter()
        .flatten()
        .map(|(id, h)| (*id, (h.preview.clone(), h.running)))
        .collect()
}

/// How long [`deliver_decision`] waits for the engine to take a verdict. The
/// engine polls its slot every 50 ms while it waits, so a live wait takes the
/// verdict within one or two ticks; anything past this means nothing is
/// waiting for it.
const DECISION_PICKUP: std::time::Duration = std::time::Duration::from_millis(500);

/// Hand a tool verdict to a session's engine and confirm it was taken.
///
/// The slot only says "written", not "accepted": a verdict for a request the
/// engine has moved past sits there until the next poll drops it as stale. So
/// the write is followed by a short wait for the engine to empty the slot. A
/// verdict still in place after that is withdrawn (it must not approve a later
/// request) and reported, which is the only definite failure. The tool's own
/// run time plays no part: the engine takes the verdict before it dispatches.
///
/// Blocking — call it without holding any lock the engine's event path needs.
pub fn deliver_decision(
    slot: &Mutex<Option<(u64, agent_kernel::tools::registry::ToolVerdict)>>,
    request_id: u64,
    approved: bool,
    action: Option<String>,
) -> Result<(), String> {
    *slot.lock().map_err(|e| e.to_string())? = Some((
        request_id,
        agent_kernel::tools::registry::ToolVerdict { approved, action },
    ));
    let deadline = std::time::Instant::now() + DECISION_PICKUP;
    loop {
        {
            let mut current = slot.lock().map_err(|e| e.to_string())?;
            let picked = current
                .as_ref()
                .is_some_and(|(id, verdict)| *id == request_id && verdict.approved == approved);
            // Cleared by the engine — the verdict was taken.
            if !picked {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                *current = None;
                return Err(format!("审批 #{request_id} 没有送达，当前没有等待中的审批"));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A session's live handles — the actor runs on its own thread; these are
/// the UI's write endpoints into it.
pub struct SessionHandle {
    #[allow(dead_code)]
    pub id: i64,
    pub cmd_tx: tokio::sync::mpsc::Sender<crate::command::UiCommand>,
    pub decision: Arc<Mutex<Option<(u64, agent_kernel::tools::registry::ToolVerdict)>>>,
    /// The approval this session is parked on right now (`0` = none), kept
    /// from the event stream by the shell's forwarder. `ToolDecision` is
    /// written into the ACTIVE session's slot, and the engine discards a stale
    /// id, so a verdict for a request that is not the one waiting must be
    /// refused up front: checking the id here turns it into an error the UI can
    /// show, instead of an approval that waits out the engine's 300 s timeout.
    pub pending_approval: Arc<std::sync::atomic::AtomicU64>,
    /// `AnswerQuestion` resolves the parked ask_question oneshot — same
    /// bypass-the-pump channel as `decision`.
    pub ask: Arc<agent_kernel::tools::registry::AskChannel>,
    pub permissions: Arc<std::sync::RwLock<agent_kernel::permissions::PermissionGate>>,
    /// The session's live agent mode — `SetAgentMode` writes here mid-turn
    /// so the next sampling round dispatches against the swapped registry.
    pub agent_mode: Arc<std::sync::RwLock<String>>,
    /// The session's live thinking level — the engine's shared slot. Read by
    /// `model_info` so the UI shows the session's actual value, which may
    /// differ from the stored default after a prefs change.
    pub thinking_level: Arc<std::sync::RwLock<Option<String>>>,
    /// The session's live `(provider, model)`. `SetModel` writes it here
    /// before the command reaches the pump, so `model_info` answers with the
    /// new pick at once instead of after the running turn drains the queue.
    pub model: Arc<std::sync::RwLock<(String, String)>>,
    pub steer_tx: tokio::sync::mpsc::Sender<String>,
    /// The session's parked-prompt deque, shared with the actor — the IPC
    /// layer mutates it directly (`Enqueue`/`SetQueued` bypass the command
    /// pump, which blocks behind a running turn) and echoes the change via
    /// `ui`; the actor drains from the same deque on `DrainQueue`/turn end.
    pub queue: Arc<Mutex<std::collections::VecDeque<String>>>,
    /// Direct cancel flag — set `true` to abort the in-flight turn. Bypasses
    /// the command pump so a Cancel isn't queued behind `run_turn`.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    pub shutdown: Arc<tokio::sync::Notify>,
    /// Sidebar preview — updated from events + turn boundaries.
    pub preview: String,
    /// True while this session's actor is mid-turn (drives the ⟳ marker).
    pub running: bool,
    /// This session's UI event queue — kept here so the shell can read the
    /// delivery counters (coalesced/dropped deltas, queue high-water mark)
    /// without reaching into the actor.
    pub ui: UiSink,
    /// The session's background jobs — listed and controlled from here, not
    /// through the pump, so a kill lands while a turn is running.
    pub bg_jobs: Arc<agent_tools_shell::bg::BgRegistry>,
    /// The entries the actor read back from the session's job record — listed
    /// ahead of the live jobs and removable from here.
    pub bg_restored: crate::session::BgRestored,
    /// The session's linked workspaces — the same handle the actor's tool
    /// context reads, so a grant applies to the next call without the pump.
    pub linked_roots: Arc<agent_capabilities::LinkedRoots>,
    /// The actor's notice channel — a mid-session grant is announced on
    /// the model's next turn through it.
    pub notices: tokio::sync::mpsc::Sender<String>,
}

/// The multi-session kernel manager — frontend-agnostic. Owns all live
/// actors' handles + the tagged event queue; the frontend keeps
/// `active_id` and renders that session's stream.
pub struct SessionManager {
    /// Where the host keeps config and workspace-local state. Supplied by the
    /// product, which knows its own name.
    paths: AppPaths,
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
    plugins: Option<crate::plugins::RuntimePluginHandle>,
    /// Plugin-declared lifecycle hooks, shared with every session's engine
    /// and rebuilt in place by `reload_plugins`.
    hooks: agent_kernel::hooks::HookChain,
    /// Sidebar metadata (persisted index + live preview overrides).
    pub metas: Vec<SessionMeta>,
    /// The session the stream is showing.
    pub active_id: i64,
    /// Active provider + model names — surfaced on the status bar.
    pub provider_name: String,
    pub model_name: String,
    /// The settings-pane default model (`active_model` in Agent 偏好) —
    /// fills the gap between a workspace's saved model and the config's
    /// own `active_*`. `None` → resolve_provider's config default.
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    /// Global subagent default `(provider, model)` — pushed into every actor
    /// so `delegate` children run on it. `None` → children follow the parent.
    pub subagent_model: Option<(String, String)>,
    /// Global subagent thinking level — pushed with `subagent_model`.
    /// `None` → children follow the parent session's level.
    pub subagent_thinking_level: Option<String>,
    /// Active thinking intensity level across sessions (e.g. "medium", "high").
    pub active_thinking_level: Option<String>,
    /// Active permission mode across sessions (e.g. "default", "acceptEdits", "auto").
    pub permission_mode: String,
    /// Active agent mode across sessions (`build` | `plan` | `goal`).
    pub agent_mode: String,
    /// Which mode the app opens in — `last` (上次使用) | `build` (编程) |
    /// `office` (工作).
    /// Mirrored from the persisted defaults so the settings pane can
    /// round-trip it; the boot decision itself already happened in
    /// `spawn_at`.
    pub default_workspace_mode: String,
    /// Fraction of the context window that triggers compaction — the
    /// settings UI's 70/80/90% choices; applies to actors spawned after set.
    pub compact_at: f32,
    /// Seconds an approval card may sit unanswered — the settings window's
    /// default for new sessions, broadcast to live ones when it changes.
    pub approval_timeout_secs: u64,
    /// The same bound for an `ask_question` card.
    pub question_timeout_secs: u64,
    /// Memory subsystem master switch — global default from the settings
    /// window; applies to sessions spawned after set.
    pub memory_enabled: bool,
    /// Model distillation switch — `false` keeps deterministic episode/fact
    /// writes but drops the per-turn summarizer model call.
    pub memory_distill: bool,
    /// Semantic recall switch — `false` scores recall on the hash vector even
    /// in a build that carries the sentence encoder.
    pub semantic_recall: bool,
    /// `.blueprint/` switch — the default for sessions spawned from now on,
    /// and broadcast to live sessions when it changes.
    pub blueprint_enabled: bool,
    /// Workspace root (for git branch / cwd display).
    pub workspace_root: std::path::PathBuf,
    /// Workspace is open. `false` = empty state: no actors, no sessions.
    pub workspace_active: bool,
}

/// Process-lifetime runtime that owns plugin (MCP) background tasks.
/// Must outlive every `load_all`: the stdio demux reader is spawned on it.
fn plugin_runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("plugin-rt")
            .enable_all()
            .build()
            .expect("plugin runtime")
    })
}

impl SessionManager {
    /// Mutable access to a session's live handle in the ACTIVE workspace
    /// (sidebar running/preview).
    pub fn handle_mut(&mut self, id: i64) -> Option<&mut SessionHandle> {
        self.handles.get_mut(&id)
    }

    /// Read-only twin of [`Self::handle_mut_at`] — a session's live handle
    /// wherever it lives (active workspace or `parked`).
    pub fn handle_at(&self, root: &str, id: i64) -> Option<&SessionHandle> {
        if self.workspace_root.to_string_lossy() == root {
            self.handles.get(&id)
        } else {
            self.parked.get(root)?.get(&id)
        }
    }

    /// A session's background-job registry, wherever its actor lives. Cloned
    /// out so the caller can drop the manager lock before a `kill` waits on
    /// the monitor. `None` for a session with no live actor — it has no jobs.
    pub fn bg_registry(
        &self,
        root: &str,
        id: i64,
    ) -> Option<Arc<agent_tools_shell::bg::BgRegistry>> {
        self.handle_at(root, id).map(|h| h.bg_jobs.clone())
    }

    /// The 后台 view's list for a session: the entries earlier processes left
    /// in its record, then its live jobs; with no actor up, just the record.
    /// A job the record still calls running died with the process that ran it
    /// — the app was closed — so it reads as interrupted rather than as work
    /// still going.
    pub fn bg_jobs_for_display(&self, root: &str, id: i64) -> serde_json::Value {
        if let Some(h) = self.handle_at(root, id) {
            return crate::session::bg_snapshot(&h.bg_restored, &h.bg_jobs);
        }
        let saved = self.bg_store(root).and_then(|s| s.load_bg_jobs(id));
        serde_json::Value::Array(saved.map(crate::session::restored_bg_jobs).unwrap_or_default())
    }

    /// Drop one restored entry from a session's job record, returning the list
    /// the pane should show. `None` when `job` is not a restored entry — a live
    /// job, which only its registry can remove.
    pub fn remove_bg_record(&self, root: &str, id: i64, job: &str) -> Option<serde_json::Value> {
        let store = self.bg_store(root)?;
        let is_job = |e: &serde_json::Value| e.get("id").and_then(|v| v.as_str()) == Some(job);
        if let Some(h) = self.handle_at(root, id) {
            {
                let mut restored = h.bg_restored.lock().ok()?;
                let before = restored.len();
                restored.retain(|e| !is_job(e));
                if restored.len() == before {
                    return None;
                }
            }
            // The registry did not change, so its feed will not write this.
            let _ = store.save_bg_jobs(id, &crate::session::bg_record(&h.bg_restored, &h.bg_jobs));
            return Some(crate::session::bg_snapshot(&h.bg_restored, &h.bg_jobs));
        }
        let mut list = crate::session::restored_bg_jobs(store.load_bg_jobs(id)?);
        list.retain(|e| !is_job(e));
        let jobs = serde_json::Value::Array(list);
        let _ = store.save_bg_jobs(id, &jobs);
        Some(jobs)
    }

    /// The store a session of `root` keeps its record in. Records sit in each
    /// workspace's own directory, so a parked workspace's session must not be
    /// looked up in the active one's.
    fn bg_store(&self, root: &str) -> Option<Arc<SessionStore>> {
        if self.workspace_root.to_string_lossy() == root {
            return Some(self.store.clone());
        }
        SessionStore::open(std::path::Path::new(root)).ok().map(Arc::new)
    }
    // ---- linked workspaces ------------------------------------------------

    /// The roots session `id` may reach beyond its workspace: what the
    /// workspace remembered plus what this session was granted, normalized.
    /// Read from the store, so it answers for a session with no live actor.
    pub fn effective_links(
        &self,
        root: &str,
        id: i64,
    ) -> Vec<agent_capabilities::LinkedRoot> {
        self.links_hub(root).effective(id)
    }

    /// The two grant layers for a session, each already normalized:
    /// `(workspace-level, session-level)`. The UI shows them apart because
    /// only one of them survives the session.
    pub fn link_layers(
        &self,
        root: &str,
        id: i64,
    ) -> (
        Vec<agent_capabilities::LinkedRoot>,
        Vec<agent_capabilities::LinkedRoot>,
    ) {
        self.links_hub(root).layers(id)
    }

    /// Grant `path` to session `id`. `persist` remembers it for every session
    /// of this workspace; otherwise it dies with this session. Returns the
    /// session's effective list.
    pub fn link_root(
        &mut self,
        root: &str,
        id: i64,
        path: &std::path::Path,
        access: &str,
        persist: bool,
    ) -> Result<Vec<agent_capabilities::LinkedRoot>, String> {
        let layer = if persist {
            crate::links::Layer::Workspace
        } else {
            crate::links::Layer::Session
        };
        self.links_hub(root)
            .grant(id, &path.to_string_lossy(), access, layer)
    }

    /// Drop a grant, whichever layer holds it. A path granted both ways is
    /// gone from both — "取消关联" means the session no longer reaches it.
    pub fn unlink_root(
        &mut self,
        root: &str,
        id: i64,
        path: &std::path::Path,
    ) -> Result<Vec<agent_capabilities::LinkedRoot>, String> {
        self.links_hub(root).revoke(id, path)
    }

    /// Switch one link between read and read-write, in whichever layer holds
    /// it (persistent first: that is the broader grant).
    pub fn set_link_access(
        &mut self,
        root: &str,
        id: i64,
        path: &std::path::Path,
        access: &str,
    ) -> Result<Vec<agent_capabilities::LinkedRoot>, String> {
        self.links_hub(root).set_access(id, path, access)
    }

    /// The hub every grant of `root` goes through — the manager's ops and a
    /// running actor's approval card share one instance, so a grant made in a
    /// background window is live in this one too, and a revocation reaches
    /// every window that was already open.
    fn links_hub(&self, root: &str) -> Arc<crate::links::LinkHub> {
        crate::links::hub_for(std::path::Path::new(root), &self.paths)
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
    pub fn spawn(paths: AppPaths) -> (Self, std_mpsc::Receiver<(String, i64, UiEvent)>) {
        Self::spawn_at(None, paths)
    }

    /// Boot the manager at a specific workspace root (or current directory / most recent).
    pub fn spawn_at(
        root: Option<std::path::PathBuf>,
        paths: AppPaths,
    ) -> (Self, std_mpsc::Receiver<(String, i64, UiEvent)>) {
        // The session database is a process-wide singleton, so its directory is
        // set once here rather than passed to every read.
        crate::office::init_store(paths.data_dir.clone());
        // Global user defaults — the settings window's stored preference.
        // Loaded up here because the「默认进入」row decides the boot root
        // below; every later layer reads this same snapshot.
        let defaults = crate::prefs::try_load_default_preferences();
        // Semantic recall is a switch on top of the build feature: the
        // embedding model is only loaded when the user asked for it, and
        // nothing about recall changes until it is up. Memory off means recall
        // off — the model is a download and a resident quarter-gigabyte, and
        // the master switch governs that cost.
        let (memory_on, semantic_on) = defaults
            .as_ref()
            .map(|d| (d.memory_enabled, d.semantic_recall))
            .unwrap_or((true, false));
        agent_context::memory::semantic::set_enabled(
            memory_on && semantic_on,
            paths.data_dir.join("models"),
        );
        // No explicit root → the boot workspace `default_workspace_mode`
        // asks for. Nothing to open → empty state (no cwd fallback — don't
        // silently attach the launch directory).
        let cwd: Option<std::path::PathBuf> = root
            .or_else(|| boot_workspace(defaults.as_ref(), &crate::prefs::load_recent_workspaces()));
        let cfg = AppConfig::load(&paths.config_dir, None).unwrap_or_default();
        let (event_tx, event_rx) = std_mpsc::channel();
        let (_p, default_model, default_pname) = resolve_provider(&cfg);

        // MCP servers: registered once, before any session exists, so the
        // session resumed below is built with the router. `load_all` bounds
        // each registration (8 s) and skips failures, and an empty plugin dir
        // is skipped without even creating a runtime.
        let hooks = agent_kernel::hooks::HookChain::new();
        let plugins = cwd.as_ref().map(|root| {
            let handle: crate::plugins::RuntimePluginHandle =
                Arc::new(std::sync::RwLock::new(None));
            // Background the load: a wedged MCP handshake must not hold up
            // boot — the router starts empty and the swap lands whenever the
            // registrations finish. A turn taken before then just sees no
            // plugin tools.
            let h = handle.clone();
            let hooks_bg = hooks.clone();
            let root = root.clone();
            let paths_bg = paths.clone();
            std::thread::Builder::new()
                .name("mcp-boot-load".into())
                .spawn(move || {
                    *h.write().unwrap() = Self::load_plugins(&root, &hooks_bg, &paths_bg, None);
                })
                .expect("mcp boot loader");
            handle
        });
        let plugins_for_manager = plugins;
        let hooks_for_manager = hooks;

        // Empty boot: inert store, no actors, no metas.
        let Some(cwd) = cwd else {
            let store = Arc::new(
                SessionStore::open_or_transient(
                    &agent_store::app_data_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp")),
                )
                .expect("initialize shared session store"),
            );
            let defaults = crate::prefs::try_load_default_preferences();
            if let Some(d) = &defaults {
                crate::sandbox_prefs::set(crate::sandbox_prefs::SandboxLimits {
                    enabled: d.sandbox_enabled,
                    network: d
                        .sandbox_network
                        .as_deref()
                        .and_then(crate::sandbox_prefs::network_from_label),
                    max_memory_mb: d.sandbox_max_memory_mb,
                    max_processes: d.sandbox_max_processes,
                    allowed_domains: d.sandbox_allowed_domains.clone().unwrap_or_default(),
                    denied_domains: d.sandbox_denied_domains.clone().unwrap_or_default(),
                    extra_ro_mounts: agent_skills::scanner::global_skill_roots(),
                    extra_rw_mounts: Vec::new(),
                });
            }
            // No workspace yet — the settings-pane default pair is the only
            // override below config.active_* that can apply here.
            let (empty_pname, empty_model) = resolve_model_pair(
                &None,
                &None,
                defaults.as_ref(),
                &cfg.providers,
                (default_pname.clone(), default_model.clone()),
            );
            let mgr = Self {
                paths: paths.clone(),
                plugins: plugins_for_manager.clone(),
                hooks: hooks_for_manager,
                metas: Vec::new(),
                store,
                provider_cfg: cfg,
                config_stamp: Self::config_stamp_now(&paths.config_dir),
                handles: HashMap::new(),
                parked: HashMap::new(),
                event_tx,
                active_id: 0,
                provider_name: empty_pname,
                model_name: empty_model,
                default_provider: defaults.as_ref().and_then(|d| d.active_provider.clone()),
                default_model: defaults.as_ref().and_then(|d| d.active_model.clone()),
                subagent_model: defaults.as_ref().and_then(|d| d.subagent_pair()),
                subagent_thinking_level: defaults
                    .as_ref()
                    .and_then(|d| d.subagent_thinking_level.clone()),
                active_thinking_level: defaults.as_ref().and_then(|d| d.thinking_level.clone()),
                permission_mode: defaults
                    .as_ref()
                    .map(|d| d.permission_mode.clone())
                    .unwrap_or_else(|| crate::prefs::DEFAULT_PERMISSION_MODE.into()),
                agent_mode: defaults
                    .as_ref()
                    .map(|d| d.agent_mode.clone())
                    .unwrap_or_else(|| "build".into()),
                default_workspace_mode: defaults
                    .as_ref()
                    .map(|d| d.default_workspace_mode.clone())
                    .unwrap_or_else(|| "last".into()),
                compact_at: defaults
                    .as_ref()
                    .map(|d| d.compact_at)
                    .unwrap_or(agent_kernel::compaction::COMPACT_AT),
                approval_timeout_secs: defaults
                    .as_ref()
                    .map(|d| d.approval_timeout_secs)
                    .unwrap_or(crate::prefs::DEFAULT_APPROVAL_TIMEOUT_SECS),
                question_timeout_secs: defaults
                    .as_ref()
                    .map(|d| d.question_timeout_secs)
                    .unwrap_or(crate::prefs::DEFAULT_QUESTION_TIMEOUT_SECS),
                memory_enabled: defaults.as_ref().map(|d| d.memory_enabled).unwrap_or(true),
                memory_distill: defaults.as_ref().map(|d| d.memory_distill).unwrap_or(true),
                semantic_recall: defaults.as_ref().is_some_and(|d| d.semantic_recall),
                blueprint_enabled: defaults
                    .as_ref()
                    .map(|d| d.blueprint_enabled)
                    .unwrap_or(true),
                workspace_root: std::path::PathBuf::new(),
                workspace_active: false,
            };
            return (mgr, event_rx);
        };

        let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
        if crate::office::is_office_root(&canon) {
            // The office dir lives under the app data dir and is created
            // lazily — a boot landing here needs it to exist.
            let _ = std::fs::create_dir_all(&canon);
        }
        let store = Arc::new(
            SessionStore::open_or_transient(&canon).expect("initialize shared session store"),
        );

        crate::prefs::record_recent_workspace(&canon);

        let prefs = workspace_layer(&canon, &store.load_prefs());
        // `defaults` (loaded at the top of `spawn_at`) fills any gap the
        // workspace prefs leave. Absent file → built-ins.
        // Model resolution order: workspace's saved pair → the settings
        // pane's default pair (Agent 偏好 active_model) → the config's
        // active_provider/active_model → first provider fallback. Every
        // layer validates the provider still exists in `providers`.
        let (provider_name, model_name) = resolve_model_pair(
            &prefs.provider,
            &prefs.model,
            defaults.as_ref(),
            &cfg.providers,
            (default_pname.clone(), default_model.clone()),
        );
        let permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| crate::prefs::DEFAULT_PERMISSION_MODE.into());
        let agent_mode = resolve_agent_mode(
            &canon,
            &prefs.agent_mode,
            defaults.as_ref().map(|d| &d.agent_mode),
        );
        let active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.as_ref().and_then(|d| d.thinking_level.clone()));
        let compact_at = defaults
            .as_ref()
            .map(|d| d.compact_at)
            .unwrap_or(agent_kernel::compaction::COMPACT_AT);
        let approval_timeout_secs = defaults
            .as_ref()
            .map(|d| d.approval_timeout_secs)
            .unwrap_or(crate::prefs::DEFAULT_APPROVAL_TIMEOUT_SECS);
        let question_timeout_secs = defaults
            .as_ref()
            .map(|d| d.question_timeout_secs)
            .unwrap_or(crate::prefs::DEFAULT_QUESTION_TIMEOUT_SECS);
        let memory_enabled = defaults.as_ref().map(|d| d.memory_enabled).unwrap_or(true);
        let memory_distill = defaults.as_ref().map(|d| d.memory_distill).unwrap_or(true);
        let semantic_recall = defaults
            .as_ref()
            .map(|d| d.semantic_recall)
            .unwrap_or(false);
        // Sandbox overrides are process-global — seed the slot once so
        // process tools read them even before the settings window opens.
        if let Some(d) = &defaults {
            crate::sandbox_prefs::set(crate::sandbox_prefs::SandboxLimits {
                enabled: d.sandbox_enabled,
                network: d
                    .sandbox_network
                    .as_deref()
                    .and_then(crate::sandbox_prefs::network_from_label),
                max_memory_mb: d.sandbox_max_memory_mb,
                max_processes: d.sandbox_max_processes,
                allowed_domains: d.sandbox_allowed_domains.clone().unwrap_or_default(),
                denied_domains: d.sandbox_denied_domains.clone().unwrap_or_default(),
                extra_ro_mounts: agent_skills::scanner::global_skill_roots(),
                extra_rw_mounts: Vec::new(),
            });
        }

        let config_stamp = Self::config_stamp_now(&paths.config_dir);
        let mut mgr = Self {
            paths,
            plugins: plugins_for_manager,
            hooks: hooks_for_manager,
            metas: store.list(),
            store,
            provider_cfg: cfg,
            config_stamp,
            handles: HashMap::new(),
            parked: HashMap::new(),
            event_tx,
            active_id: 0,
            provider_name,
            model_name,
            default_provider: defaults.as_ref().and_then(|d| d.active_provider.clone()),
            default_model: defaults.as_ref().and_then(|d| d.active_model.clone()),
            subagent_model: defaults.as_ref().and_then(|d| d.subagent_pair()),
            subagent_thinking_level: defaults
                .as_ref()
                .and_then(|d| d.subagent_thinking_level.clone()),
            active_thinking_level,
            permission_mode,
            agent_mode,
            default_workspace_mode: defaults
                .as_ref()
                .map(|d| d.default_workspace_mode.clone())
                .unwrap_or_else(|| "last".into()),
            compact_at,
            approval_timeout_secs,
            question_timeout_secs,
            memory_enabled,
            memory_distill,
            semantic_recall,
            blueprint_enabled: defaults
                .as_ref()
                .map(|d| d.blueprint_enabled)
                .unwrap_or(true),
            workspace_root: canon,
            workspace_active: true,
        };

        // Resume the session the user last had open; fall back to the
        // most recent when it no longer exists, or start fresh.
        let first = mgr
            .store
            .last_active()
            .filter(|id| mgr.metas.iter().any(|m| m.id == *id))
            .or_else(|| mgr.metas.iter().find(|m| !m.archived).map(|m| m.id));
        match first {
            Some(id) => mgr.open_session(id),
            // Auto-create only for a workspace that has NEVER had a session —
            // one the user emptied by deleting them all stays empty until
            // they make a new one themselves.
            None => {
                if !mgr.store.ever_used() {
                    mgr.new_session();
                }
            }
        }
        (mgr, event_rx)
    }

    /// Switch to another workspace directory: opens that workspace's session store,
    /// sets active workspace, records in recents, and loads or creates its session.
    pub fn switch_workspace(&mut self, new_root: std::path::PathBuf) -> std::io::Result<()> {
        // The office pseudo-workspace lives under the app data dir and is
        // created lazily on first entry — canonicalize + the workspace
        // scan need a real directory.
        if crate::office::is_office_root(&new_root) {
            let _ = std::fs::create_dir_all(&new_root);
        }
        let canon = new_root.canonicalize().unwrap_or_else(|_| new_root.clone());
        let store = Arc::new(SessionStore::open(&canon)?);
        let prefs = workspace_layer(&canon, &store.load_prefs());
        let defaults = crate::prefs::try_load_default_preferences();
        let (_p, default_model, default_pname) = resolve_provider(&self.provider_cfg);
        // Same chain as spawn: workspace pair → Agent 偏好 default →
        // config active_* → first provider.
        let (provider_name, model_name) = resolve_model_pair(
            &prefs.provider,
            &prefs.model,
            defaults.as_ref(),
            &self.provider_cfg.providers,
            (default_pname, default_model),
        );
        self.default_provider = defaults.as_ref().and_then(|d| d.active_provider.clone());
        self.default_model = defaults.as_ref().and_then(|d| d.active_model.clone());
        self.permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| crate::prefs::DEFAULT_PERMISSION_MODE.into());
        self.agent_mode = resolve_agent_mode(
            &canon,
            &prefs.agent_mode,
            defaults.as_ref().map(|d| &d.agent_mode),
        );
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
        crate::prefs::record_recent_workspace(&self.workspace_root);

        let first = self
            .store
            .last_active()
            .filter(|id| self.metas.iter().any(|m| m.id == *id))
            .or_else(|| self.metas.iter().find(|m| !m.archived).map(|m| m.id));
        match first {
            Some(id) => self.open_session(id),
            // Same never-used gate as spawn — and a workspace that was
            // emptied by deletes leaves `active_id` cleared, not pointed
            // at the previous workspace's session.
            None => {
                self.active_id = 0;
                if !self.store.ever_used() {
                    self.new_session();
                }
            }
        }
        Ok(())
    }

    /// List recent workspaces from the persistent recent_workspaces store.
    pub fn recent_workspaces(&self) -> Vec<crate::prefs::RecentWorkspace> {
        crate::prefs::load_recent_workspaces()
    }

    /// Drop a project from recents and tear down its actors (parked or
    /// active — a running turn is aborted). Session files stay on disk:
    /// reopening the folder restores history. Returns true when the ACTIVE
    /// workspace went away.
    pub fn remove_workspace(&mut self, root: &str) -> bool {
        let canon = std::path::Path::new(root)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(root));
        // The office pseudo-workspace is built-in — it can't be removed
        // from recents or torn down; its sessions live under its own
        // `office` namespace.
        if crate::office::is_office_root(&canon) {
            return false;
        }
        let key = canon.to_string_lossy().into_owned();
        if let Some(handles) = self.parked.remove(&key) {
            for h in handles.values() {
                h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        crate::prefs::remove_recent_workspace(&canon);
        // The workspace's Serena server (if any) goes with it — a closed
        // project should not keep an indexed child process around.
        agent_tools_serena::shutdown(&canon);

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
    /// The office pseudo-workspace is always present — it's the sidebar's
    ///「办公」section, the entry point into office mode. Its sessions are
    /// listed (and openable) from every mode: reading the list never
    /// requires switching into it first.
    ///
    /// The active workspace is read from the *store* rather than
    /// `self.metas` — metas only refresh on structural ops and go stale the
    /// moment an actor persists a turn — and is overlaid with the live
    /// `running`/`active` flags from the handles. Other workspaces are
    /// store-only snapshots: only one workspace has live actors at a time.
    pub fn projects_overview(&self) -> Vec<ProjectOverview> {
        self.projects_snapshot().build()
    }

    /// The manager-side half of [`Self::projects_overview`]: paths, recency
    /// and the live actor overlay — no session index is read here. A shell
    /// that holds the manager behind a lock takes this under it and calls
    /// [`ProjectsSnapshot::build`] after letting go.
    pub fn projects_snapshot(&self) -> ProjectsSnapshot {
        let canon = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());
        let office_root = crate::office::office_root().map(|p| p.canonicalize().unwrap_or(p));
        let office_is_current =
            self.workspace_active && office_root.as_ref().is_some_and(|o| *o == canon);
        let recents = self.recent_workspaces();

        let mut out = Vec::new();
        if self.workspace_active {
            out.push(SnapshotProject {
                root: canon.to_string_lossy().into_owned(),
                name: project_name(&canon),
                last_opened: 0,
                current: true,
                office: office_is_current,
                store: StoreRef::Open(self.store.clone()),
                live: live_overlay(Some(&self.handles)),
                active_id: self.active_id,
            });
        }

        // Office entry — pinned right after the current project. When office
        // IS the current workspace the entry above already carries it (with
        // `current: true`), so nothing is injected twice.
        if !office_is_current {
            if let Some(o) = office_root.as_ref() {
                let key = o.to_string_lossy().into_owned();
                let last_opened = recents
                    .iter()
                    .find(|w| w.path.canonicalize().unwrap_or_else(|_| w.path.clone()) == *o)
                    .map(|w| w.last_opened)
                    .unwrap_or(0);
                out.push(SnapshotProject {
                    live: live_overlay(self.parked.get(&key)),
                    root: key,
                    name: project_name(o),
                    last_opened,
                    current: false,
                    office: true,
                    store: StoreRef::Existing(o.clone()),
                    active_id: 0,
                });
            }
        }

        for w in recents {
            let w_canon = w.path.canonicalize().unwrap_or_else(|_| w.path.clone());
            // Office came out as its own entry above — a recents row for the
            // same path would double-list it as a regular project.
            if office_root.as_ref().is_some_and(|o| *o == w_canon) {
                continue;
            }
            if self.workspace_active && w_canon == canon {
                // Already first — borrow its recency stamp instead of
                // listing the same project twice.
                out[0].last_opened = w.last_opened;
                continue;
            }
            // Parked handles = this workspace's actors are still live —
            // overlay their real running/preview so the sidebar orb stays
            // on a turn the user switched away from mid-flight.
            let root = w_canon.to_string_lossy().into_owned();
            out.push(SnapshotProject {
                live: live_overlay(self.parked.get(&root)),
                root,
                name: w.name,
                last_opened: w.last_opened,
                current: false,
                office: false,
                store: StoreRef::Existing(w_canon),
                active_id: 0,
            });
        }
        ProjectsSnapshot { projects: out }
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
            names.insert(agent_store::workspace_key(&w.path), w.name.clone());
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
                .and_then(|r| names.get(&agent_store::workspace_key(r)).cloned())
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
                    "cache_write": u.cache_write,
                    "context_window": u.context_window,
                    "model": m.model,
                    "provider": m.provider,
                    // The post-turn distillation pass is a side channel: its
                    // spend never enters `usage`, so a reader must not have to
                    // add it to the session's token count.
                    "distill_calls": m.distill.map(|d| d.calls).unwrap_or(0),
                    "distill_prompt": m.distill.map(|d| d.prompt).unwrap_or(0),
                    "distill_completion": m.distill.map(|d| d.completion).unwrap_or(0),
                }));
            }
        }
        out
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
        // Office sessions are always office mode — the picker isn't offered
        // in the office workspace at all, so a stale per-session value from
        // before office became workspace-scoped can't survive a respawn.
        let sess_agent_mode = if crate::office::is_office_root(&self.workspace_root) {
            "office".to_string()
        } else {
            meta.as_ref()
                .and_then(|m| m.agent_mode.clone())
                .unwrap_or_else(|| self.agent_mode.clone())
        };
        // `thinking_level` is itself an Option: `and_then` flattens
        // `Option<Option<String>>` so a session that never set one (meta
        // present, field None) still falls back to the workspace default —
        // a `.map` here would unwrap to `Some(None)` and drop the fallback.
        let sess_thinking = meta
            .as_ref()
            .and_then(|m| m.thinking_level.clone())
            .or_else(|| self.active_thinking_level.clone());

        // Per-session cache-write accounting. The provider publishes
        // Anthropic's cache_creation tokens here; the checkpoint relay and the
        // turn-end persistence read the same counter.
        let usage_observer = Arc::new(agent_llm::UsageObserver::default());
        // One identity for this conversation, shared by the session provider,
        // the fallback chain and every rebuild the actor does later.
        let conversation = crate::session::conversation_key(id, &self.workspace_root);
        // Never substitute another vendor here: an unbuildable session provider
        // fails its turns with the reason instead (see `choose_session_provider`).
        let (provider, unavailable) = choose_session_provider(
            &self.provider_cfg,
            &sess_provider,
            Some(&usage_observer),
            Some(&conversation),
        );
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
            paths: self.paths.clone(),
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
            approval_timeout: Some(std::time::Duration::from_secs(
                self.approval_timeout_secs,
            )),
            question_timeout: Some(std::time::Duration::from_secs(
                self.question_timeout_secs,
            )),
            memory_enabled: self.memory_enabled,
            memory_distill: self.memory_distill,
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
        actor.set_usage_observer(usage_observer.clone());

        // Say WHY the session cannot answer, once, before the first turn. The
        // turn itself fails with the same reason, so this is the early notice.
        if let Some(reason) = &unavailable {
            let _ = channels
                .event_tx
                .send(agent_ipc::UiEvent::SystemMessage(format!(
                    "无法使用该会话的模型提供商: {reason}"
                )));
        }
        let cmd_tx = actor.command_sender();
        let decision = actor.decision_writer();
        let ask = actor.ask_channel();
        let permissions = actor.permissions_writer();
        let agent_mode = actor.agent_mode_writer();
        let thinking_level = actor.thinking_writer();
        let model_slot = actor.model_writer();
        let steer_tx = actor.steer_writer();
        let cancel = actor.cancel_writer();
        let shutdown = actor.shutdown_writer();
        let queue = actor.queue_writer();
        let bg_jobs = actor.bg_registry();
        let bg_restored = actor.bg_restored();
        let linked_handle = actor.linked_roots();
        let notices = actor.notice_writer();

        // Lifecycle hooks, installed before the actor starts: the shared chain
        // may gain hooks later (`reload_plugins`) without a restart.
        actor.set_hooks(self.hooks.clone());
        actor.set_secrets(self.provider_cfg.resolved_secrets());
        actor.set_fallbacks(
            &sess_provider,
            build_fallback_chain(
                &self.provider_cfg,
                &sess_provider,
                Some(&usage_observer),
                Some(&conversation),
            ),
        );
        actor.set_blueprint_enabled(self.blueprint_enabled);

        // Actor run() on its own tokio thread — a background session keeps
        // running its turn while the UI shows another session.
        // Panic-safety: a panic inside `actor.run()` unwinds `block_on` and
        // silently kills the thread — `cmd_rx` then reports Closed and every
        // later prompt vanishes with zero UI feedback. Catch the unwind so
        // the crash surfaces as an event instead of a dead session.
        // NOTE: only effective under `panic = "unwind"` (dev/test). The
        // release profile sets `panic = "abort"`, where an actor panic ends
        // the whole process instead of reaching this handler.
        let crash_sink = channels.event_tx.clone();
        std::thread::Builder::new()
            .name(format!("session-{id}-rt"))
            .spawn(move || {
                // `Runtime::new()` sizes the worker pool to the core count;
                // with one runtime per live session that multiplies into
                // hundreds of idle threads. Two workers are enough for one
                // actor (turn task + cancel/approval watchers).
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rt.block_on(async move { actor.run().await });
                })) {
                    let msg = e
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                        .unwrap_or_else(|| "unknown panic".into());
                    tracing::error!(session = id, panic = %msg, "session actor crashed");
                    let _ = crash_sink
                        .send(agent_ipc::UiEvent::Error(format!("会话执行器崩溃: {msg}")));
                    let _ = crash_sink.send(agent_ipc::UiEvent::StateChanged(
                        agent_ipc::AgentState::Idle,
                    ));
                }
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
                    // Pure `recv → send` pump: no worker pool needed.
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio rt");
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
                pending_approval: Default::default(),
                ask,
                permissions,
                agent_mode,
                thinking_level,
                model: model_slot,
                steer_tx,
                queue,
                cancel,
                shutdown,
                preview,
                running: false,
                ui,
                bg_jobs,
                bg_restored,
                linked_roots: linked_handle,
                notices,
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
            ..Default::default()
        });
        self.metas = self.store.list();
        self.spawn_actor(id);
        self.active_id = id;
        // Remember it like open/fork do, so a crash while working in a fresh
        // session restores this one instead of the previously opened session.
        let _ = self.store.set_last_active(id);
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
        // `self.metas` only refreshes on structural ops — a live session's
        // `persist_settings`/`persist_turn` lands on disk without touching
        // the cache, so spawning/mirroring off the stale copy restores the
        // session's PRE-swap model. Read fresh.
        self.metas = self.store.list();
        // The manager's model / provider / mode / level fields are the
        // PREVIOUS session's mirror. Falling back to them for a session that
        // never pinned a field is how one session's change leaked into
        // another ("switched the model here and the other session changed
        // too"): `spawn_actor` and `mirror_active_settings` both use `self.*`
        // as the "workspace default". Re-resolve that default first; the
        // session's own pins and live slots override it below.
        self.restore_workspace_default_settings();
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
                self.agent_mode = m.clone();
            }
            if let Ok(l) = h.thinking_level.read() {
                self.active_thinking_level = l.clone();
            }
            if let Ok(pm) = h.model.read() {
                let (p, m) = pm.clone();
                if !p.is_empty() {
                    self.provider_name = p;
                }
                if !m.is_empty() {
                    self.model_name = m;
                }
            }
        }
    }

    /// Re-resolve the WORKSPACE default settings (workspace prefs → global
    /// defaults → built-ins) into `self.*`. Called before spawning a NEW
    /// session: a fresh session's template is the workspace default, and
    /// `self.*` may currently be mirroring another session's per-session
    /// choice. Same precedence as `spawn_at`/`switch_workspace`.
    fn restore_workspace_default_settings(&mut self) {
        let prefs = workspace_layer(&self.workspace_root, &self.store.load_prefs());
        let defaults = crate::prefs::try_load_default_preferences();
        let (_p, default_model, default_pname) = resolve_provider(&self.provider_cfg);
        // Same three-layer chain as spawn_at/switch_workspace — the previous
        // hand-rolled match skipped the Agent 偏好 layer entirely, so a
        // settings-pane default could never reach a *new* session (only the
        // boot path honored it).
        let (provider_name, model_name) = resolve_model_pair(
            &prefs.provider,
            &prefs.model,
            defaults.as_ref(),
            &self.provider_cfg.providers,
            (default_pname, default_model),
        );
        self.provider_name = provider_name;
        self.model_name = model_name;
        self.permission_mode = prefs
            .permission_mode
            .or_else(|| defaults.as_ref().map(|d| d.permission_mode.clone()))
            .unwrap_or_else(|| crate::prefs::DEFAULT_PERMISSION_MODE.into());
        self.agent_mode = resolve_agent_mode(
            &self.workspace_root,
            &prefs.agent_mode,
            defaults.as_ref().map(|d| &d.agent_mode),
        );
        self.active_thinking_level = prefs
            .thinking_level
            .or_else(|| defaults.and_then(|d| d.thinking_level));
    }

    /// Delete a session — abort its turn, drop the actor handle (closing
    /// the command channel ends its loop), and remove the store data.
    /// If it was active, switch to the most recent remaining session —
    /// when none are left the workspace stays empty (`active_id = 0`)
    /// until the user creates one.
    pub fn delete_session(&mut self, id: i64) {
        if !self.workspace_active {
            return;
        }
        if let Err(error) = self.store.remove(id) {
            tracing::error!(%error, session=id, "session deletion failed");
            return;
        }
        if let Some(h) = self.handles.remove(&id) {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            h.shutdown.notify_one();
        }
        self.metas = self.store.list();
        if self.active_id == id {
            match self.metas.iter().find(|m| !m.archived).map(|m| m.id) {
                Some(next) => self.open_session(next),
                // Deleting the last session leaves the workspace empty —
                // no auto-created replacement; the user makes the next
                // one explicitly.
                None => self.active_id = 0,
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
        let (runs, tasks) = crate::session::history_counts(&hist);
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
            // Pins, archive and the custom-title flag stay with the source —
            // a fork is a new conversation, not a duplicate of its placement.
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
            // stay with the source session. Run/task counts describe that
            // copied history.
            queued_prompts: Vec::new(),
            runs: Some(runs),
            tasks: Some(tasks),
            ..Default::default()
        });
        self.metas = self.store.list();
        self.spawn_actor(new_id);
        self.active_id = new_id;
        let _ = self.store.set_last_active(new_id);
        Some(new_id)
    }

    /// Discover + register every MCP server for `root`. `None` when there is
    /// nothing to load, so an empty plugin dir costs no runtime.
    ///
    /// The plugins' lifecycle hooks are installed into `hooks` — the shared
    /// chain every session's engine holds a clone of.
    fn load_plugins(
        root: &std::path::Path,
        hooks: &agent_kernel::hooks::HookChain,
        paths: &AppPaths,
        prev: Option<&agent_plugin::PluginManager>,
    ) -> Option<Arc<agent_plugin::PluginManager>> {
        if agent_plugin::discover(root, paths).is_empty() {
            hooks.replace(Vec::new());
            return None;
        }
        // The MCP stdio reader/stderr tasks are `tokio::spawn`ed on whichever
        // runtime runs `load_all`. A runtime dropped after the handshake
        // kills them, leaving clients that list tools but never see a
        // response. Run on a process-lifetime runtime instead.
        let mgr = plugin_runtime().block_on(agent_plugin::reload_all(
            root,
            paths,
            &crate::sandbox_prefs::protected_paths(paths),
            prev,
        ));
        hooks.replace(crate::plugins::manifest_hooks(mgr.hook_specs()));
        tracing::info!(
            tools = mgr.exported_tools().len(),
            hooks = hooks.len(),
            "MCP plugins loaded"
        );
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
            Some(handle) => Self::reload_plugins_detached(&(
                handle.clone(),
                self.hooks.clone(),
                self.workspace_root.clone(),
                self.paths.clone(),
            )),
            None => Vec::new(),
        }
    }

    /// Snapshot the shared plugin plumbing — `plugins` handle, hook chain,
    /// workspace root — so the IPC layer can drop the manager guard BEFORE a
    /// reload. `load_all` runs seconds of subprocess/MCP handshake I/O per
    /// plugin; holding the manager mutex through it dead-queued every other
    /// op (workspace open, session switch, UI stats) behind it.
    pub fn plugin_reload_parts(
        &self,
    ) -> Option<(
        crate::plugins::RuntimePluginHandle,
        agent_kernel::hooks::HookChain,
        std::path::PathBuf,
        AppPaths,
    )> {
        self.plugins.clone().map(|h| {
            (
                h,
                self.hooks.clone(),
                self.workspace_root.clone(),
                self.paths.clone(),
            )
        })
    }

    /// `reload_plugins` over a snapshot — same swap, no manager borrow.
    pub fn reload_plugins_detached(
        parts: &(
            crate::plugins::RuntimePluginHandle,
            agent_kernel::hooks::HookChain,
            std::path::PathBuf,
            AppPaths,
        ),
    ) -> Vec<serde_json::Value> {
        let (handle, hooks, root, paths) = parts;
        // Hold the old manager across the reload so unchanged servers keep
        // their live connection; it drops (killing leftovers) after the swap.
        let prev = handle.read().unwrap().clone();
        let loaded = Self::load_plugins(root, hooks, paths, prev.as_deref());
        let summary = loaded.as_ref().map(|m| m.status()).unwrap_or_default();
        *handle.write().unwrap() = loaded;
        summary
    }

    /// `trust_plugin` over a snapshot — the grant is persisted BEFORE the
    /// reload so the load path (which reads grants from disk) sees it.
    pub fn trust_plugin_detached(
        parts: &(
            crate::plugins::RuntimePluginHandle,
            agent_kernel::hooks::HookChain,
            std::path::PathBuf,
            AppPaths,
        ),
        id: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        // Hash the manifest as it is on disk right now: the grant must cover
        // exactly what the loader will run next.
        let manifest = agent_plugin::current_manifest(id, &parts.2, &parts.3)
            .ok_or_else(|| format!("no manifest for plugin `{id}` in this workspace"))?;
        let path = agent_plugin::trust_store_path(&parts.3);
        let mut store = agent_plugin::TrustStore::load(&path);
        store.trust_content(id, &parts.2, &manifest);
        store.save(&path);
        Ok(Self::reload_plugins_detached(parts))
    }

    /// `set_plugin_enabled` over a snapshot — flag write then reload, off
    /// the manager lock.
    pub fn set_plugin_enabled_detached(
        parts: &(
            crate::plugins::RuntimePluginHandle,
            agent_kernel::hooks::HookChain,
            std::path::PathBuf,
            AppPaths,
        ),
        id: &str,
        enabled: bool,
    ) -> Result<Vec<serde_json::Value>, String> {
        let path = agent_plugin::disabled_store_path(&parts.3);
        let mut store = agent_plugin::DisabledStore::load(&path);
        store.set(id, !enabled);
        store.save(&path);
        Ok(Self::reload_plugins_detached(parts))
    }

    /// The `/` picker's plugin half — enabled plugins' declared slash
    /// commands, flattened for the composer (discovery order, manifest
    /// data — not the live connection, same contract as `status()`).
    pub fn plugin_commands(&self) -> Vec<serde_json::Value> {
        self.plugins
            .as_ref()
            .and_then(|h| h.read().ok().and_then(|g| g.clone()))
            .map(|m| {
                m.commands()
                    .into_iter()
                    .map(|c| {
                        serde_json::json!({
                            "name": c.name,
                            "qualified": c.qualified(),
                            "pluginId": c.plugin_id,
                            "description": c.description,
                            "action": c.action,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
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
    pub async fn mcp_probe(
        workspace_root: &std::path::Path,
        id: &str,
        paths: &AppPaths,
    ) -> serde_json::Value {
        let Some(mpath) = agent_plugin::discover(workspace_root, paths)
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
        match agent_plugin::McpClient::start(
            &manifest,
            paths.app_name(),
            &crate::sandbox_prefs::protected_paths(paths),
        )
        .await
        {
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

    /// Grant a repo-local plugin consent to load (settings UI). Without this
    /// its manifest is discovered but never registered — and a plugin hook is
    /// local code, so an untrusted checkout must not run one.
    ///
    /// The grant is persisted (`plugin-trust.json`) and the live router is
    /// rebuilt so the plugin loads without an app restart.
    pub fn trust_plugin(&self, id: &str) -> Result<Vec<serde_json::Value>, String> {
        if self.plugins.is_none() {
            return Err("plugin router not installed for this workspace".into());
        }
        // Record the grant first, then reload: the load path reads grants from
        // disk, so persisting before it runs is what makes the plugin load.
        let manifest = agent_plugin::current_manifest(id, &self.workspace_root, &self.paths)
            .ok_or_else(|| format!("no manifest for plugin `{id}` in this workspace"))?;
        let path = agent_plugin::trust_store_path(&self.paths);
        let mut store = agent_plugin::TrustStore::load(&path);
        store.trust_content(id, &self.workspace_root, &manifest);
        store.save(&path);
        Ok(self.reload_plugins())
    }

    /// Toggle a plugin/MCP's enabled flag — persists to `plugin-state.json`,
    /// then reloads so its hooks stop firing and its tools stop advertising
    /// without an app restart. Same write-then-reload shape as `trust_plugin`:
    /// the load path reads the store from disk.
    pub fn set_plugin_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<Vec<serde_json::Value>, String> {
        if self.plugins.is_none() {
            return Err("plugin router not installed for this workspace".into());
        }
        let path = agent_plugin::disabled_store_path(&self.paths);
        let mut store = agent_plugin::DisabledStore::load(&path);
        store.set(id, !enabled);
        store.save(&path);
        Ok(self.reload_plugins())
    }

    /// Delete a discovered plugin's directory (settings UI). The path comes
    /// from discovery, so an id can never point outside the plugins dirs.
    pub fn remove_plugin(
        workspace_root: &std::path::Path,
        id: &str,
        paths: &AppPaths,
    ) -> Result<(), String> {
        let mpath = agent_plugin::discover(workspace_root, paths)
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
    /// `manifest.json` files under `~/.config/husk/{plugins,mcp}/` and
    /// `<repo>/.husk/{plugins,mcp}/`. Listing only: this never spawns a server.
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
        for mpath in agent_plugin::discover(&self.workspace_root, &self.paths) {
            let Ok(text) = std::fs::read_to_string(&mpath) else {
                continue;
            };
            let Ok(mut m) = serde_json::from_str::<agent_plugin::PluginManifest>(&text) else {
                continue;
            };
            // `dir` is serde-skipped — refill it the way `load_all` does, or
            // `repoLocal`/`dir` below are computed off an empty path.
            m.dir = mpath.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let id = m.id.clone();
            let st = live.get(&id);
            let field = |k: &str| st.map(|v| v[k].clone()).unwrap_or(serde_json::Value::Null);
            // Repo-local manifests are inert until trusted — the card has to
            // be able to say so, and to show what a trusted plugin would run.
            let mgr = self
                .plugins
                .as_ref()
                .and_then(|h| h.read().ok().and_then(|g| g.clone()));
            let trusted = agent_plugin::current_manifest(&id, &self.workspace_root, &self.paths)
                .zip(mgr.as_ref())
                .is_some_and(|(manifest, m)| {
                    m.trust
                        .is_trusted_content(&id, &self.workspace_root, &manifest)
                });
            // The toggle binds the PERSISTED intent, not the live flag — a
            // failed or untrusted plugin reads "on" (it would run once the
            // blocker clears), because live `enabled` being false there says
            // nothing about what the user asked for.
            let enabled = mgr.as_ref().map(|m| !m.is_disabled(&id)).unwrap_or(true);
            let repo_local = m
                .dir
                .starts_with(self.paths.workspace_dir(&self.workspace_root));
            // Declared slash commands — name + action, so a tool/wasm plugin
            // card can show what it contributes instead of only its hooks.
            let commands: Vec<serde_json::Value> = m
                .capabilities
                .commands
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "qualified": format!("{id}:{}", c.name),
                        "description": c.description,
                        "action": c.action,
                    })
                })
                .collect();
            // A plugin that never loaded has no live status — and a
            // TRUST-REFUSED one lands in `errors`, where `status()` emits
            // `hooks: []`. Treat both as "fall back to the manifest" so the
            // card still shows the hooks that WOULD run.
            let hooks = match field("hooks") {
                live if live.as_array().map(|a| !a.is_empty()).unwrap_or(false) => live,
                _ => serde_json::json!(m
                    .capabilities
                    .hooks
                    .iter()
                    .map(|h| serde_json::json!({
                        "event": h.event,
                        "command": h
                            .run
                            .as_ref()
                            .map(|r| {
                                std::iter::once(r.command.clone())
                                    .chain(r.args.iter().cloned())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_default(),
                    }))
                    .collect::<Vec<_>>()),
            };
            // Same fallback for tools — an unloaded or failed plugin still
            // shows what it WOULD export (manifest `capabilities.tools`).
            let tools = match field("tools") {
                live if live.as_array().map(|a| !a.is_empty()).unwrap_or(false) => live,
                _ => serde_json::json!(m
                    .capabilities
                    .tools
                    .iter()
                    .map(|t| serde_json::json!(t.name))
                    .collect::<Vec<_>>()),
            };
            out.push(serde_json::json!({
                "id": id,
                "name": m.name,
                "version": m.version,
                // `null` for a pure plugin — `kind` only exists when there is
                // an `entry` for it to describe.
                "kind": m.kind.map(|k| match k {
                    agent_plugin::PluginKind::Mcp => "mcp",
                    agent_plugin::PluginKind::Wasm => "wasm",
                }),
                "entry": m.entry,
                // Live tool names when connected; manifest `capabilities.tools`
                // names otherwise (an unloaded wasm/declarative plugin still
                // shows what it exports).
                "tools": tools,
                "commands": m.capabilities.commands.len(),
                // Full declared command list — `commands` stays a count for
                // existing readers; the card renders names from this.
                "commandList": commands,
                "sandboxed": m.sandboxed,
                "dir": m.dir.to_string_lossy(),
                "connected": field("connected"),
                "serverVersion": field("serverVersion"),
                "error": field("error"),
                // Lifecycle hooks the plugin declares — live list when it
                // connected, the manifest's own declaration otherwise.
                "hooks": hooks,
                "trusted": trusted,
                "repoLocal": repo_local,
                // Persisted toggle intent — NOT the live `enabled` (which is
                // also false for load failures and trust refusals).
                "enabled": enabled,
            }));
        }
        out
    }

    /// The two user-instruction files the settings "指令" pane edits — the
    /// host's user-wide `AGENTS.md` and `<workspace>/AGENTS.md` (project).
    /// Both append to every new session's system prompt.
    pub fn instructions(&self) -> serde_json::Value {
        let global = Some(self.paths.config_file("AGENTS.md"));
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
            "global" => self.paths.config_file("AGENTS.md"),
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

    /// Steer one subagent of the active session. The owner is `active_id`, read
    /// here on the server; the caller names only the child.
    pub fn steer_active_child(&self, child_id: u64, text: String) -> Result<(), String> {
        agent_kernel::tools::delegate::steer_child(self.active_id, child_id, text)
    }

    /// The running subagents of one session, with the label, task and question
    /// a redrawn card needs. A webview that reloads mid-turn loses every event
    /// that drew the cards, and history keeps no row for a child still running,
    /// so the kernel's live registry is the only description left.
    ///
    /// Empty for a session with no live actor: a child cannot outlive the
    /// process, and a parked workspace's session of the same number must not be
    /// read through the active workspace's manager.
    pub fn active_children(
        &self,
        root: &str,
        id: i64,
    ) -> Vec<agent_kernel::tools::delegate::ActiveChild> {
        if self.handle_at(root, id).is_none() {
            return Vec::new();
        }
        agent_kernel::tools::delegate::active_child_infos(id)
    }

    /// Load a session's persisted history (for rebuilding a closed view).
    pub fn store_history(&self, id: i64) -> Option<Vec<agent_llm::types::ChatMessage>> {
        self.store.load_history(id)
    }

    /// The active workspace's session store, for reads that should not hold
    /// the manager's lock — [`SessionStore::tool_results`] walks a whole
    /// journal's index. `None` with no workspace open.
    pub fn active_store(&self) -> Option<Arc<SessionStore>> {
        self.workspace_active.then(|| self.store.clone())
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
    ) -> std::io::Result<(Vec<agent_llm::types::ChatMessage>, usize, usize, usize)> {
        if !self.workspace_active {
            return Ok((Vec::new(), 0, 0, 0));
        }
        self.store.load_history_page(id, before, count)
    }

    /// Trace pagination must use one immutable history revision, including
    /// recovery snapshots whose last message can change without growing.
    pub fn store_history_page_checked(
        &self,
        id: i64,
        before: Option<usize>,
        count: usize,
        expected_revision: Option<&str>,
    ) -> std::io::Result<agent_store::HistoryPageSnapshot> {
        if !self.workspace_active {
            return Err(std::io::Error::other("请先选择工作区"));
        }
        self.store
            .load_history_page_checked(id, before, count, expected_revision)
    }

    /// A session's persisted meta — usage meter, run/task counts and parked
    /// prompts seed a rebuilt view (before any new event). Reads the store
    /// fresh: `self.metas` only refreshes on structural ops and goes stale
    /// the moment an actor persists a turn. One per-id read, not a scan.
    pub fn store_meta(&self, id: i64) -> Option<SessionMeta> {
        if !self.workspace_active {
            return None;
        }
        self.store.meta(id)
    }

    /// Persist a session's queued-prompt list into its `SessionMeta` — used
    /// by the IPC fast path (`Enqueue`/`SetQueued` write the shared deque
    /// directly), where the actor's own `emit_queued` can't be reached until
    /// the pump frees up — possibly a whole turn later.
    pub fn set_session_queued(&mut self, id: i64, items: Vec<String>) {
        if !self.workspace_active {
            return;
        }
        if let Err(error) = self.store.update_queue(id, items.clone()) {
            tracing::error!(%error, session=id, "queued prompts persistence failed");
            return;
        }
        if let Some(m) = self.metas.iter_mut().find(|m| m.id == id) {
            m.queued_prompts = items;
        }
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

    /// Pin/unpin a session within its workspace. `root` names the workspace
    /// (`None` = the active one) — see [`Self::edit_session_at`].
    pub fn pin_session(
        &mut self,
        root: Option<&str>,
        id: i64,
        pinned: bool,
    ) -> std::io::Result<bool> {
        self.edit_session_at(root, |s| s.set_pinned(id, pinned))
    }

    /// Global pin/unpin — the cross-project 会话 list's top tier.
    pub fn pin_session_global(
        &mut self,
        root: Option<&str>,
        id: i64,
        pinned: bool,
    ) -> std::io::Result<bool> {
        self.edit_session_at(root, |s| s.set_pinned_global(id, pinned))
    }

    /// Archive/restore. Archiving drops both pins (see `set_archived`).
    pub fn archive_session(
        &mut self,
        root: Option<&str>,
        id: i64,
        archived: bool,
    ) -> std::io::Result<bool> {
        self.edit_session_at(root, |s| s.set_archived(id, archived))
    }

    /// User title; blank resets to the title derived from the first prompt.
    pub fn rename_session(
        &mut self,
        root: Option<&str>,
        id: i64,
        title: &str,
    ) -> std::io::Result<bool> {
        self.edit_session_at(root, |s| s.rename(id, title))
    }

    /// One meta edit on the store of `root`'s workspace.
    ///
    /// These edits only touch a session's index row, so unlike fork/delete
    /// (which own live actors) they work on any known workspace: the sidebar
    /// lists every project's sessions and each row gets the same menu. A
    /// parked actor of that workspace committing a turn later keeps the edit
    /// (`commit_turn` preserves user-owned fields).
    fn edit_session_at(
        &mut self,
        root: Option<&str>,
        f: impl FnOnce(&SessionStore) -> std::io::Result<bool>,
    ) -> std::io::Result<bool> {
        match self.session_target(root) {
            SessionTarget::Active => {
                let ok = f(&self.store)?;
                if ok {
                    self.metas = self.store.list();
                }
                Ok(ok)
            }
            SessionTarget::Other(store) => f(&store),
            SessionTarget::Missing => Ok(false),
        }
    }

    /// Which store a sidebar op on `root` addresses.
    fn session_target(&self, root: Option<&str>) -> SessionTarget {
        let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let active = self.workspace_active.then(|| canon(&self.workspace_root));
        match root.filter(|r| !r.is_empty()) {
            None => match active {
                Some(_) => SessionTarget::Active,
                None => SessionTarget::Missing,
            },
            Some(r) => {
                let target = canon(std::path::Path::new(r));
                if active.as_ref() == Some(&target) {
                    SessionTarget::Active
                } else {
                    SessionStore::open_existing(&target)
                        .map(SessionTarget::Other)
                        .unwrap_or(SessionTarget::Missing)
                }
            }
        }
    }

    /// The session as a Markdown transcript — what「导出记录」saves. `None`
    /// when the session has no history yet.
    pub fn export_session_markdown(&self, root: Option<&str>, id: i64) -> Option<(String, String)> {
        let other;
        let store: &SessionStore = match self.session_target(root) {
            SessionTarget::Active => &self.store,
            SessionTarget::Other(s) => {
                other = s;
                &other
            }
            SessionTarget::Missing => return None,
        };
        let history = store.load_history(id)?;
        let title = store
            .list()
            .into_iter()
            .find(|m| m.id == id)
            .map(|m| m.title)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("会话 {id}"));
        Some((title.clone(), session_markdown(&title, id, &history)))
    }

    /// mtime of the config file — the gate for a disk reload.
    fn config_stamp_of(
        path: Option<&std::path::Path>,
        config_dir: &std::path::Path,
    ) -> Option<std::time::SystemTime> {
        path.map(|p| p.to_path_buf())
            .or_else(|| Some(AppConfig::default_path(config_dir)))
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
    }

    fn config_stamp_now(config_dir: &std::path::Path) -> Option<std::time::SystemTime> {
        Self::config_stamp_of(None, config_dir)
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
        let stamp = Self::config_stamp_of(path, &self.paths.config_dir);
        if stamp.is_none() || stamp == self.config_stamp {
            return false;
        }
        let Ok(cfg) = AppConfig::load(&self.paths.config_dir, path) else {
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
    /// away). The command carries no pair: each actor re-reads the fresh config
    /// for ITS OWN provider/model — broadcasting the mirror once re-pointed
    /// every session at the active one's model. Returns `(notified, live)`.
    fn notify_live_actors(&self) -> (usize, usize) {
        let cmd = crate::command::UiCommand::ReloadModel;
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
                    // Resolve overrides exactly as session spawn/hot-swap does.
                    let d = pcfg.model_opts(m.id());
                    let reasoning = d.reasoning.unwrap_or(false) || d.thinking_level_map.is_some();
                    let thinking_level_map = d.thinking_level_map.clone();
                    let available_levels =
                        agent_llm::config::available_thinking_levels(thinking_level_map.as_ref());

                    models.push(ModelDetails {
                        provider: pname.clone(),
                        model: m.id().to_string(),
                        name: Some(m.name().to_string()),
                        reasoning,
                        thinking_level_map,
                        available_levels,
                        context_window: d.context_window,
                        max_tokens: d.max_tokens.map(|v| v.min(u32::MAX as u64) as u32),
                        cost: d.cost.clone(),
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

        // `active_*` must describe the ACTIVE SESSION — `SetModel` persists
        // the pick to the session's own meta row, not the manager's
        // workspace default, so reporting `self.model_name` here names the
        // model the session was SPAWNED with and every model_info refresh
        // snaps the pickers/chips back to it. Same rule as the live
        // permission/thinking/agent-mode reads below. The live slot comes
        // first: the meta row only catches up once the actor drains the
        // `SetModel`, which waits behind a running turn.
        let live_model = self
            .active()
            .and_then(|h| h.model.read().ok().map(|g| g.clone()))
            .filter(|(p, m)| !p.is_empty() && !m.is_empty());
        let (active_provider, active_model) = live_model.unwrap_or_else(|| {
            self.store
                .list()
                .into_iter()
                .find(|m| m.id == self.active_id)
                .map(|m| (m.provider, m.model))
                .map(|(p, m)| {
                    (
                        p.unwrap_or_else(|| self.provider_name.clone()),
                        m.unwrap_or_else(|| self.model_name.clone()),
                    )
                })
                .unwrap_or_else(|| (self.provider_name.clone(), self.model_name.clone()))
        });

        if !models
            .iter()
            .any(|m| m.provider == active_provider && m.model == active_model)
        {
            models.insert(
                0,
                ModelDetails {
                    provider: active_provider.clone(),
                    model: active_model.clone(),
                    name: Some(active_model.clone()),
                    reasoning: false,
                    thinking_level_map: None,
                    available_levels: Vec::new(),
                    context_window: None,
                    max_tokens: None,
                    cost: None,
                },
            );
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
                let a = h.agent_mode.read().ok().map(|m| m.clone());
                (m, t, a)
            })
            .unwrap_or((None, None, None));

        let requested_level = live_level.or_else(|| self.active_thinking_level.clone());
        let active_map = models
            .iter()
            .find(|model| model.provider == active_provider && model.model == active_model)
            .and_then(|model| model.thinking_level_map.as_ref());
        let active_thinking_level =
            agent_llm::config::normalize_thinking_level(requested_level.as_deref(), active_map);

        let path = Some(
            AppConfig::default_path(&self.paths.config_dir)
                .to_string_lossy()
                .to_string(),
        );
        SessionModelInfo {
            active_provider,
            active_model,
            active_thinking_level,
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
        // The STORED row, not `self.*`: `permission_mode`,
        // `active_thinking_level` and `agent_mode` on the manager mirror the
        // ACTIVE SESSION (`mirror_active_settings`). Returning those made the
        // 设置 page display whichever session was in front, and the next save
        // wrote that session's values back as the "default".
        let stored = crate::prefs::try_load_default_preferences().unwrap_or_default();
        (
            stored.permission_mode,
            stored.thinking_level,
            stored.agent_mode,
            stored.compact_at,
        )
    }

    /// The settings-pane waiting bounds — `(approval_secs, question_secs)`,
    /// `0` meaning "wait forever". Read from the STORED row for the same
    /// reason as `default_prefs`: the manager's own copies follow the live
    /// session once one is mirrored in.
    pub fn timeout_prefs(&self) -> (u64, u64) {
        let stored = crate::prefs::try_load_default_preferences().unwrap_or_default();
        (stored.approval_timeout_secs, stored.question_timeout_secs)
    }

    /// The「默认进入」preference — `last` (上次使用) | `build` (编程) |
    /// `office` (工作). Only the next launch obeys it: the current boot
    /// already picked its workspace.
    pub fn default_workspace_mode(&self) -> String {
        self.default_workspace_mode.clone()
    }

    /// The `default_preferences` row every setter writes back.
    ///
    /// `permission_mode`, `thinking_level` and `agent_mode` on `self` mirror the
    /// ACTIVE SESSION (`mirror_active_settings`) — they are not the user's saved
    /// defaults, so they come from the stored row, never from `self`. Saving
    /// them from `self` meant that changing 「默认进入」, the default model or a
    /// memory switch silently overwrote the saved defaults with whatever
    /// session happened to be in front. Every other field follows `self` (or
    /// the process-wide sandbox slot); `set_default_prefs` then patches the
    /// three fields it was explicitly given.
    fn defaults_for_save(&self) -> crate::prefs::DefaultPreferences {
        let stored = crate::prefs::try_load_default_preferences().unwrap_or_default();
        let lim = crate::sandbox_prefs::current();
        crate::prefs::DefaultPreferences {
            permission_mode: stored.permission_mode,
            thinking_level: stored.thinking_level,
            agent_mode: stored.agent_mode,
            default_workspace_mode: self.default_workspace_mode.clone(),
            compact_at: self.compact_at,
            approval_timeout_secs: self.approval_timeout_secs,
            question_timeout_secs: self.question_timeout_secs,
            active_provider: self.default_provider.clone(),
            active_model: self.default_model.clone(),
            subagent_provider: self.subagent_model.as_ref().map(|(p, _)| p.clone()),
            subagent_model: self.subagent_model.as_ref().map(|(_, m)| m.clone()),
            subagent_thinking_level: self.subagent_thinking_level.clone(),
            sandbox_enabled: lim.enabled,
            sandbox_network: Some(lim.network_label().into()),
            sandbox_max_memory_mb: lim.max_memory_mb,
            sandbox_max_processes: lim.max_processes,
            sandbox_allowed_domains: (!lim.allowed_domains.is_empty())
                .then(|| lim.allowed_domains.clone()),
            sandbox_denied_domains: (!lim.denied_domains.is_empty())
                .then(|| lim.denied_domains.clone()),
            memory_enabled: self.memory_enabled,
            memory_distill: self.memory_distill,
            semantic_recall: self.semantic_recall,
            blueprint_enabled: self.blueprint_enabled,
            // Not the manager's to own — the notification switch belongs to
            // the shell, so a save from here carries the stored value over.
            desktop_notifications: stored.desktop_notifications,
            notification_sound: stored.notification_sound,
            schema_version: crate::prefs::DEFAULT_PREFERENCES_VERSION,
        }
    }

    /// Store the「默认进入」preference. Boot-only — no live session or
    /// workspace is touched; the value takes effect at the next launch.
    pub fn set_default_workspace_mode(&mut self, mode: String) {
        self.default_workspace_mode = mode;
        let _ = crate::prefs::save_default_preferences(&self.defaults_for_save());
    }

    /// Update the stored default preferences. Permission mode, thinking
    /// level and agent mode are per-session composer choices, so live
    /// sessions are untouched by those; the compaction ratio has no
    /// per-session override, so it IS pushed into every live actor (see
    /// `broadcast_compact_at`).
    pub fn set_default_prefs(
        &mut self,
        permission_mode: Option<String>,
        thinking_level: Option<String>,
        agent_mode: Option<String>,
        compact_at: Option<f32>,
        approval_timeout_secs: Option<u64>,
        question_timeout_secs: Option<u64>,
    ) {
        if let Some(f) = compact_at {
            self.compact_at = f.clamp(0.5, 0.95);
            // The ratio is a workspace-wide rule with no per-session
            // override — a running session must not keep the threshold it
            // was spawned with.
            self.broadcast_compact_at(self.compact_at);
        }
        if let Some(s) = approval_timeout_secs {
            self.approval_timeout_secs = s;
        }
        if let Some(s) = question_timeout_secs {
            self.question_timeout_secs = s;
        }
        if approval_timeout_secs.is_some() || question_timeout_secs.is_some() {
            // Same reach as the compaction ratio: a live session keeps the
            // bounds it was born with unless the change is pushed into it.
            self.broadcast_timeouts();
        }
        // Write the DEFAULTS row only. `self.permission_mode` /
        // `active_thinking_level` / `agent_mode` mirror the ACTIVE SESSION, so
        // assigning them here re-pointed the live session's displayed settings
        // at the new default (its actual gate never changed) and left the
        // 设置 pane reading a value that followed the session. Only the fields
        // THIS call set may be written: the pane sends a single-field patch.
        let mut prefs = self.defaults_for_save();
        if let Some(m) = &permission_mode {
            prefs.permission_mode = m.clone();
        }
        if let Some(l) = &thinking_level {
            prefs.thinking_level = if l.is_empty() { None } else { Some(l.clone()) };
        }
        if let Some(m) = &agent_mode {
            prefs.agent_mode = m.clone();
        }
        if let Some(f) = compact_at {
            prefs.compact_at = f.clamp(0.5, 0.95);
        }
        if let Some(s) = approval_timeout_secs {
            prefs.approval_timeout_secs = s;
        }
        if let Some(s) = question_timeout_secs {
            prefs.question_timeout_secs = s;
        }
        let _ = crate::prefs::save_default_preferences(&prefs);
        // A workspace pin outranks the default at spawn, so drop the pinned
        // fields the user just re-defaulted HERE — "new sessions use X" has to
        // mean exactly that, the same treatment `set_default_model` gives the
        // model pair. Other workspaces keep theirs.
        let _ = self.store.update_prefs(|ws| {
            if permission_mode.is_some() {
                ws.permission_mode = None;
            }
            if thinking_level.is_some() {
                ws.thinking_level = None;
            }
            if agent_mode.is_some() {
                ws.agent_mode = None;
            }
        });
    }

    /// Memory subsystem switches — `(enabled, distill)`. Both are
    /// app-global defaults read at spawn: a change applies to sessions
    /// spawned after it (live actors keep their wiring).
    pub fn memory_prefs(&self) -> (bool, bool) {
        (self.memory_enabled, self.memory_distill)
    }

    /// The blueprint default — unlike memory, a change is broadcast to live
    /// sessions by `set_blueprint_enabled`, so this is the settings echo.
    pub fn blueprint_pref(&self) -> bool {
        self.blueprint_enabled
    }

    /// Whether recall is scored by the sentence encoder — the settings echo.
    pub fn semantic_recall_pref(&self) -> bool {
        self.semantic_recall
    }

    /// Flip semantic recall. Takes effect on the next recall, in every
    /// session: the model sits behind one process-wide switch, so nothing has
    /// to be re-spawned. Enabling starts the background model load.
    pub fn set_semantic_recall(&mut self, on: bool) {
        self.semantic_recall = on;
        self.apply_semantic_switch();
        let _ = crate::prefs::save_default_preferences(&self.defaults_for_save());
    }

    /// The encoder runs only when the build carries it, the user asked for it
    /// and memory itself is on — the two switches are one gate, not two.
    fn apply_semantic_switch(&self) {
        agent_context::memory::semantic::set_enabled(
            self.memory_enabled && self.semantic_recall,
            self.paths.data_dir.join("models"),
        );
    }

    /// The settings-pane default model (`active_provider`, `active_model`)
    /// — what workspaces with no saved pair spawn with. `None` on either
    /// side means "follow the config's own active_*".
    pub fn default_model(&self) -> (Option<String>, Option<String>) {
        (self.default_provider.clone(), self.default_model.clone())
    }

    /// Update + persist the default model pair. `None` clears the override;
    /// live sessions keep their live model (the default shapes NEW spawns
    /// only — same contract as every other Agent 偏好 row).
    pub fn set_default_model(&mut self, provider: Option<String>, model: Option<String>) {
        // A pair is only meaningful whole — callers clear both or set both.
        self.default_provider = provider;
        self.default_model = model;
        let _ = crate::prefs::save_default_preferences(&self.defaults_for_save());
        // A workspace pin (`prefs.provider`/`prefs.model`, written whenever a
        // model is picked in the composer) silently outranks the pair the
        // user just chose as the default — from their view "new sessions use
        // X" must mean exactly that. Drop THIS workspace's pin so the new
        // default reaches the next session here; other workspaces keep
        // theirs, and any model picked from the composer re-pins it.
        let _ = self.store.update_prefs(|prefs| {
            prefs.provider = None;
            prefs.model = None;
        });
        self.restore_workspace_default_settings();
    }

    /// The global subagent default `(provider, model)` — `None` means
    /// children follow their parent session's model.
    pub fn subagent_model(&self) -> Option<(String, String)> {
        self.subagent_model.clone()
    }

    /// Update + persist the subagent default and push it into every live and
    /// parked actor, so the next `delegate` anywhere uses it. `None` clears it.
    pub fn set_subagent_model(&mut self, pair: Option<(String, String)>) {
        self.subagent_model = pair.filter(|(p, m)| !p.is_empty() && !m.is_empty());
        let _ = crate::prefs::save_default_preferences(&self.defaults_for_save());
        self.broadcast_subagent_defaults();
    }

    /// Global thinking level for delegated children. `None` (or empty) means
    /// each session decides: children follow the parent's own level, and a
    /// manifest `thinking:` still wins over both.
    pub fn set_subagent_thinking_level(&mut self, level: Option<String>) {
        self.subagent_thinking_level = level.filter(|l| !l.trim().is_empty());
        let _ = crate::prefs::save_default_preferences(&self.defaults_for_save());
        self.broadcast_subagent_defaults();
    }

    /// Push both subagent defaults into every live and parked actor — the same
    /// reason `broadcast_memory_prefs` exists: a settings change otherwise only
    /// reached the next spawn, and a session already running keeps delegating.
    fn broadcast_subagent_defaults(&self) {
        let (provider, model) = self.subagent_model.clone().unwrap_or_default();
        let level = self.subagent_thinking_level.clone().unwrap_or_default();
        let live = self
            .handles
            .values()
            .chain(self.parked.values().flat_map(|m| m.values()));
        for h in live {
            let _ = h
                .cmd_tx
                .try_send(crate::command::UiCommand::SetSubagentModel {
                    provider: provider.clone(),
                    model: model.clone(),
                });
            let _ = h
                .cmd_tx
                .try_send(crate::command::UiCommand::SetSubagentThinking {
                    level: level.clone(),
                });
        }
    }

    /// Update + persist the blueprint switch. It becomes the default for new
    /// sessions and is pushed to every live and parked session, so each one
    /// follows it from its next turn — the same reach the old global had.
    pub fn set_blueprint_enabled(&mut self, v: bool) {
        self.blueprint_enabled = v;
        let live = self
            .handles
            .values()
            .chain(self.parked.values().flat_map(|m| m.values()));
        for h in live {
            let _ = h
                .cmd_tx
                .try_send(crate::command::UiCommand::SetBlueprintEnabled { enabled: v });
        }
        // A no-op memory update re-saves the whole prefs row — which now
        // carries the flag.
        self.set_memory_prefs(None, None);
    }

    /// Update + persist the memory switches. No live-session broadcast —
    /// the store handle and summarizer are fixed at spawn, so the new
    /// values reach only the next actor.
    pub fn set_memory_prefs(&mut self, enabled: Option<bool>, distill: Option<bool>) {
        if let Some(v) = enabled {
            self.memory_enabled = v;
        }
        if let Some(v) = distill {
            self.memory_distill = v;
        }
        // Turning memory off turns the encoder off with it, and turning it
        // back on restores what the semantic switch already said.
        self.apply_semantic_switch();
        let _ = crate::prefs::save_default_preferences(&self.defaults_for_save());
    }

    /// Push the current memory switches into every LIVE actor — same
    /// reason `broadcast_compact_at` exists: a settings change only
    /// reached the next spawn otherwise, and parked sessions keep writing
    /// episodes/facts under their birth wiring.
    pub fn broadcast_memory_prefs(&self) {
        let (enabled, distill) = self.memory_prefs();
        let live = self
            .handles
            .values()
            .chain(self.parked.values().flat_map(|m| m.values()));
        for handle in live {
            let _ = handle
                .cmd_tx
                .try_send(crate::command::UiCommand::SetMemory { enabled, distill });
        }
    }

    /// Push the compaction ratio into every LIVE actor — the value is
    /// otherwise only read at spawn, so a settings change left running
    /// sessions on the threshold they were born with. Parked workspaces are
    /// included: their actors keep draining turns in the background and must
    /// compact under the same rule.
    pub fn broadcast_compact_at(&mut self, fraction: f32) {
        let frac = fraction.clamp(0.5, 0.95);
        let live = self
            .handles
            .values()
            .chain(self.parked.values().flat_map(|m| m.values()));
        for handle in live {
            let _ = handle
                .cmd_tx
                .try_send(crate::command::UiCommand::SetCompactAt { fraction: frac });
        }
    }

    /// Push the waiting bounds into every LIVE actor — same reason as
    /// `broadcast_compact_at`: the value is otherwise only read at spawn, so a
    /// settings change would leave running sessions on the bounds they were
    /// born with.
    pub fn broadcast_timeouts(&self) {
        let (approval_secs, question_secs) =
            (self.approval_timeout_secs, self.question_timeout_secs);
        let live = self
            .handles
            .values()
            .chain(self.parked.values().flat_map(|m| m.values()));
        for handle in live {
            let _ = handle
                .cmd_tx
                .try_send(crate::command::UiCommand::SetTimeouts {
                    approval_secs,
                    question_secs,
                });
        }
    }

    /// Pin ONE workspace preference, leaving the others as stored.
    ///
    /// Writing every field from `self.*` turned a model pick into a pin on
    /// permission / mode / level too — and `self.*` mirrors the active
    /// session, so those pins were whatever session happened to be in front.
    /// It also re-pinned, on the next composer change, the fields
    /// `set_default_prefs` had just cleared so a new default could apply.
    fn pin_workspace_pref(&self, f: impl FnOnce(&mut agent_store::WorkspacePrefs)) {
        // Locked read-modify-write: the settings pane, the composer and a
        // grant can all land here, and a bare load+save drops one of them.
        let _ = self.store.update_prefs(f);
    }

    /// Update active model & provider in manager metadata and pin the pair
    /// for this workspace.
    pub fn set_model(&mut self, provider: String, model: String) {
        self.provider_name = provider.clone();
        self.model_name = model.clone();
        self.pin_workspace_pref(|p| {
            p.provider = Some(provider);
            p.model = Some(model);
        });
    }

    /// Update active thinking level in manager metadata and pin it for this
    /// workspace (`""` clears the pin).
    pub fn set_thinking_level(&mut self, level: String) {
        self.active_thinking_level = if level.is_empty() { None } else { Some(level) };
        let level = self.active_thinking_level.clone();
        self.pin_workspace_pref(|p| p.thinking_level = level);
    }

    /// Update active permission mode in manager metadata and pin it for this
    /// workspace.
    pub fn set_permission_mode(&mut self, mode: String) {
        self.permission_mode = mode.clone();
        self.pin_workspace_pref(|p| p.permission_mode = Some(mode));
    }

    /// Update active agent mode in manager metadata and persist to
    /// workspace prefs — the session's live swap happens through the
    /// `SetAgentMode` command the caller also pumps in.
    pub fn set_agent_mode(&mut self, mode: String) {
        self.agent_mode = mode.clone();
        self.pin_workspace_pref(|p| p.agent_mode = Some(mode));
    }
}

/// A readable transcript: user/assistant turns as sections, tool calls and
/// results as fenced blocks. Hidden/system rows (the kernel prompt, injected
/// nudges, compaction bookkeeping) are not part of the conversation and are
/// left out; error notices stay, since they explain gaps in the transcript.
fn session_markdown(title: &str, id: i64, history: &[agent_llm::types::ChatMessage]) -> String {
    use agent_llm::types::{NoticeKind, Role};
    let mut out = format!("# {title}\n\n> 会话 ID: {id}\n");
    for m in history {
        let text = m.content.as_deref().unwrap_or("").trim();
        match m.role {
            Role::User => {
                if matches!(m.notice, Some(NoticeKind::Hidden)) || text.is_empty() {
                    continue;
                }
                out.push_str(&format!("\n## 用户\n\n{text}\n"));
            }
            Role::Assistant => {
                if !text.is_empty() {
                    out.push_str(&format!("\n## 助手\n\n{text}\n"));
                }
                for call in m.tool_calls.iter().flatten() {
                    out.push_str(&format!(
                        "\n**工具调用** `{}`\n\n```json\n{}\n```\n",
                        call.name,
                        call.arguments.trim()
                    ));
                }
            }
            Role::Tool => {
                let label = if m.is_error == Some(true) {
                    "工具结果（失败）"
                } else {
                    "工具结果"
                };
                out.push_str(&format!(
                    "\n<details><summary>{label}</summary>\n\n```\n{text}\n```\n\n</details>\n"
                ));
            }
            Role::System => {
                if matches!(m.notice, Some(NoticeKind::Error) | Some(NoticeKind::System))
                    && !text.is_empty()
                {
                    out.push_str(&format!("\n> {text}\n"));
                } else if m.notice == Some(NoticeKind::Host) {
                    // The status line, as the stream drew it; the log tail
                    // was for the model.
                    if let Some(line) = text.lines().next().filter(|l| !l.is_empty()) {
                        out.push_str(&format!("\n> {line}\n"));
                    }
                }
            }
        }
    }
    out
}

/// Resolve the active provider from `config.toml`.
/// Model resolution order for spawn/switch: a workspace's own saved
/// provider/model pair wins; then the settings-pane default pair (Agent
/// 偏好 `active_model`); then the caller's `resolve_provider` fallback,
/// which itself consults config.toml's `active_provider`/`active_model`
/// and finally the first buildable provider. Every layer validates the
/// provider still exists in `providers` — a deleted provider falls
/// through instead of stranding the session on a dead backend.
/// The per-workspace preference layer, minus the office pseudo-workspace.
////// Office is a mode, not a project: every office session shares one root, and
/// the composer setters (`set_model` & co.) pin model / mode / level there on
/// every change. A pin saved by one office session would then outrank the Agent 偏好
/// defaults for every *later* office session — a new office session silently
/// ignoring the default model. Office therefore has no workspace layer at all;
/// the global defaults govern. (Agent mode is separate: `resolve_agent_mode`
/// pins it to `office` for this root regardless.)
fn workspace_layer(
    root: &std::path::Path,
    prefs: &agent_store::WorkspacePrefs,
) -> agent_store::WorkspacePrefs {
    if crate::office::is_office_root(root) {
        agent_store::WorkspacePrefs::default()
    } else {
        prefs.clone()
    }
}

/// The line the model reads after a grant changed. A permission is worth
/// stating plainly: which roots, what access, and the one rule that keeps
/// "relative path" unambiguous.
pub(crate) fn links_notice(roots: &[agent_capabilities::LinkedRoot]) -> String {
    let mut text = String::from("工作区许可已更新（立即生效）：\n");
    if roots.is_empty() {
        text.push_str("  （当前没有关联工作区）\n");
    }
    for r in roots {
        let access = match r.access {
            agent_capabilities::RootAccess::ReadWrite => "读写",
            agent_capabilities::RootAccess::Read => "只读",
        };
        text.push_str(&format!("  - {}（{access}）\n", r.path.display()));
    }
    text.push_str("相对路径仍然相对主工作区，访问关联目录请用绝对路径。");
    text
}

fn resolve_model_pair(
    ws_provider: &Option<String>,
    ws_model: &Option<String>,
    defaults: Option<&crate::prefs::DefaultPreferences>,
    providers: &std::collections::HashMap<String, agent_llm::ProviderConfig>,
    fallback: (String, String),
) -> (String, String) {
    if let (Some(p), Some(m)) = (ws_provider, ws_model) {
        if providers.contains_key(p) {
            return (p.clone(), m.clone());
        }
    }
    if let Some(d) = defaults {
        if let (Some(p), Some(m)) = (&d.active_provider, &d.active_model) {
            if providers.contains_key(p) {
                return (p.clone(), m.clone());
            }
        }
    }
    fallback
}

/// The workspace's default agent mode. Office is pinned to the office
/// pseudo-workspace — it's switched from the sidebar as an app-level mode,
/// not picked per session in the composer, so a stored `office` value on a
/// regular project (workspace prefs / global defaults) falls back to
/// `build` instead of leaking document sessions into a code workspace.
fn resolve_agent_mode(
    root: &std::path::Path,
    prefs_mode: &Option<String>,
    defaults_mode: Option<&String>,
) -> String {
    if crate::office::is_office_root(root) {
        return "office".into();
    }
    match prefs_mode.as_ref().or(defaults_mode) {
        Some(m) if m != "office" => m.clone(),
        _ => "build".into(),
    }
}

/// Where to boot when the caller named no root — the「默认进入」choice.
/// `office` (工作) opens the office pseudo-workspace; `build` (编程) the most
/// recent real project, skipping office (a mode, not a project — booting
/// into it records it as the newest recent, so taking the head verbatim
/// would ignore the setting); `last` (上次使用) the MRU head as-is, which is
/// what an untouched install has always done. `None` → empty state.
fn boot_workspace(
    defaults: Option<&crate::prefs::DefaultPreferences>,
    recents: &[crate::prefs::RecentWorkspace],
) -> Option<std::path::PathBuf> {
    match defaults.map(|d| d.default_workspace_mode.as_str()) {
        // The office dir is created lazily by `spawn_at` — a boot landing
        // there needs nothing pre-existing.
        Some("office") => {
            crate::office::office_root().or_else(|| recents.first().map(|w| w.path.clone()))
        }
        Some("build") => recents
            .iter()
            .map(|w| w.path.clone())
            .find(|p| p.is_dir() && !crate::office::is_office_root(p)),
        _ => recents.iter().map(|w| w.path.clone()).find(|p| p.is_dir()),
    }
}

/// The degrade chain for a session pinned to `primary`: `fallback_chain` in
/// config order, minus the primary itself (a loop), minus entries that are
/// missing or cannot be built. A broken backup is skipped rather than kept as
/// a placeholder, because the point of the chain is to find one that answers.
///
/// Every entry is built under the session's own `conversation`, so a hop to a
/// backup keeps the conversation's routing key instead of cold-starting it.
pub(crate) fn build_fallback_chain(
    cfg: &AppConfig,
    primary: &str,
    observer: Option<&Arc<agent_llm::UsageObserver>>,
    conversation: Option<&str>,
) -> Vec<(String, Arc<dyn agent_llm::LlmProvider>)> {
    cfg.fallback_chain
        .iter()
        .filter(|name| name.as_str() != primary)
        .filter_map(|name| {
            let pcfg = cfg.providers.get(name)?;
            let p = ProviderFactory::build_with_conversation(pcfg, observer.cloned(), conversation)
                .ok()?;
            Some((name.clone(), p))
        })
        .collect()
}

/// The provider instance a session runs on, for the provider it was pinned to.
///
/// A session never borrows another vendor's provider: when its own provider is
/// missing from the config or cannot be built, the returned provider fails
/// every turn with the reason. Silently routing that turn to whichever provider
/// `resolve_provider` finds sends the conversation to a backend the user did
/// not choose, under a model id that backend does not serve.
///
/// `conversation` is the session's routing identity; see
/// [`crate::session::conversation_key`].
pub(crate) fn choose_session_provider(
    cfg: &AppConfig,
    session_provider: &str,
    observer: Option<&Arc<agent_llm::UsageObserver>>,
    conversation: Option<&str>,
) -> (Arc<dyn agent_llm::LlmProvider>, Option<String>) {
    let Some(pcfg) = cfg.providers.get(session_provider) else {
        let reason = format!("provider `{session_provider}` is not in the config");
        return (
            Arc::new(agent_llm::UnavailableProvider::new(reason.clone())),
            Some(reason),
        );
    };
    match ProviderFactory::build_with_conversation(pcfg, observer.cloned(), conversation) {
        Ok(p) => (p, None),
        Err(e) => {
            let reason = format!("provider `{session_provider}`: {e}");
            (
                Arc::new(agent_llm::UnavailableProvider::new(reason.clone())),
                Some(reason),
            )
        }
    }
}

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

    /// Throwaway app paths — a test must never write to the real user dirs.
    fn test_paths() -> AppPaths {
        let base = std::env::temp_dir().join(format!("husk-test-{}", std::process::id()));
        AppPaths::new(base.join("config"), base.join("data"), ".husk")
    }

    /// The same throwaway paths, carrying a real `config.toml` that declares a
    /// `devin` provider. Tests about model metadata must not depend on the
    /// developer's own `~/.config/husk/config.toml`.
    fn test_paths_with_config() -> AppPaths {
        let paths = test_paths();
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        // Every test in this process shares `config_dir`, so a plain `write`
        // (truncate, then fill) lets a concurrent `AppConfig::load` read an
        // empty file and fall back to "no provider configured". Write aside
        // and rename: readers see the old or the new file, never a torn one.
        let tmp = paths
            .config_dir
            .join(format!("config.toml.{:?}.tmp", std::thread::current().id()));
        std::fs::write(
            &tmp,
            r#"active_provider = "devin"
active_model = "devin/swe-2"

[providers.devin]
type = "openai_responses"
base_url = "https://example.invalid/v1"
api_key = "sk-test"
default_model = "devin/swe-2"

[providers.devin.compat]
supports_reasoning_effort = true

[[providers.devin.models]]
id = "devin/swe-2"
name = "SWE-2"
reasoning = true
input = ["text", "image"]
context_window = 262144

[providers.devin.models.thinking_level_map]
off = "off"
medium = "medium"
high = "high"
max = "max"
"#,
        )
        .unwrap();
        std::fs::rename(&tmp, paths.config_dir.join("config.toml")).unwrap();
        paths
    }

    /// A config edit must reach the actors that are already running — an actor
    /// keeps the provider instance and model parameters it was spawned with, and
    /// a model that the edit removed must not stay the session's target.
    #[test]
    fn config_reload_reaches_live_actors_and_re_resolves_a_removed_model() {
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), test_paths());

        // Stand-in for a live session actor — only its command queue matters.
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(4);
        let (ui, _ui_rx) = agent_kernel::channels::UiSink::channel();
        mgr.handles.insert(
            i64::MAX,
            SessionHandle {
                id: i64::MAX,
                cmd_tx,
                decision: Arc::new(Mutex::new(None)),
                pending_approval: Default::default(),
                ask: Arc::new(agent_kernel::tools::registry::AskChannel::new(None)),
                permissions: Arc::new(std::sync::RwLock::new(
                    agent_kernel::permissions::PermissionGate::from_mode_str("default"),
                )),
                agent_mode: Arc::new(std::sync::RwLock::new("build".to_string())),
                thinking_level: Arc::new(std::sync::RwLock::new(None)),
                model: Arc::default(),
                steer_tx: tokio::sync::mpsc::channel(1).0,
                queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                shutdown: Arc::new(tokio::sync::Notify::new()),
                preview: String::new(),
                running: false,
                ui,
                bg_jobs: Default::default(),
                bg_restored: Default::default(),
                linked_roots: Default::default(),
                notices: tokio::sync::mpsc::channel(1).0,
            },
        );

        // The edit keeps one provider but renames its only model.
        let cfg: AppConfig = serde_json::from_value(serde_json::json!({
            "active_provider": "devin",
            "providers": {
                "devin": {
                    "kind": "openai_completions",
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
            if let crate::command::UiCommand::ReloadModel = cmd {
                sent.push(());
            }
        }
        // One ReloadModel on this actor's queue — no payload, each actor
        // re-reads the fresh config for its own provider/model.
        assert_eq!(sent.len(), 1);
    }

    /// One `config.toml` writer for the reload tests.
    fn write_config(path: &std::path::Path, provider: &str, model: &str) {
        std::fs::write(
            path,
            format!(
                "active_provider = \"{provider}\"\n\
                 [providers.{provider}]\n\
                 kind = \"openai_completions\"\n\
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
        // Private config dir: the boot stamp is the mtime of `config_dir`'s
        // `config.toml`, and the shared one can be rewritten by a sibling test
        // within the same mtime tick — the "changed file loads" check then
        // sees an equal stamp and skips the load.
        let paths = AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), paths);

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

    /// A session pinned to a provider that cannot be built must NOT be answered
    /// by a different vendor: the turn fails with the real reason instead.
    #[test]
    fn unbuildable_session_provider_never_falls_back_to_another_vendor() {
        std::env::remove_var("CHOOSE_PROVIDER_MISSING_KEY");
        let mut providers = HashMap::new();
        providers.insert(
            "openai".to_string(),
            agent_llm::ProviderConfig {
                kind: agent_llm::ProviderKind::OpenaiCompat,
                base_url: "https://api.openai.com/v1".into(),
                api_key: "env:CHOOSE_PROVIDER_MISSING_KEY".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "deepseek".to_string(),
            agent_llm::ProviderConfig {
                kind: agent_llm::ProviderKind::OpenaiCompat,
                base_url: "https://api.deepseek.com".into(),
                api_key: "plaintext-deepseek-key".into(),
                ..Default::default()
            },
        );
        let cfg = AppConfig {
            providers,
            ..Default::default()
        };

        let (provider, reason) = choose_session_provider(&cfg, "openai", None, None);
        assert_eq!(
            provider.id(),
            "unavailable",
            "must not borrow another vendor"
        );
        let reason = reason.expect("the failure reason must be carried");
        assert!(reason.contains("CHOOSE_PROVIDER_MISSING_KEY"), "{reason}");
    }

    /// A session naming a provider that the config no longer has is reported
    /// as unavailable too, rather than silently adopting a different one.
    #[test]
    fn deleted_session_provider_is_unavailable_not_reassigned() {
        let mut providers = HashMap::new();
        providers.insert(
            "deepseek".to_string(),
            agent_llm::ProviderConfig {
                kind: agent_llm::ProviderKind::OpenaiCompat,
                base_url: "https://api.deepseek.com".into(),
                api_key: "plaintext-deepseek-key".into(),
                ..Default::default()
            },
        );
        let cfg = AppConfig {
            providers,
            ..Default::default()
        };
        let (provider, reason) = choose_session_provider(&cfg, "gone", None, None);
        assert_eq!(provider.id(), "unavailable");
        assert!(reason.expect("reason").contains("gone"));
    }

    /// The chain keeps config order, never includes the primary itself (that
    /// would loop), and skips entries that are missing or cannot be built.
    #[test]
    fn fallback_chain_keeps_order_and_skips_primary_and_broken_entries() {
        std::env::remove_var("CHAIN_MISSING_KEY");
        let ok = |key: &str| agent_llm::ProviderConfig {
            kind: agent_llm::ProviderKind::OpenaiCompat,
            base_url: "https://x".into(),
            api_key: key.into(),
            ..Default::default()
        };
        let mut providers = HashMap::new();
        providers.insert("primary".to_string(), ok("plain-primary-key"));
        providers.insert("second".to_string(), ok("plain-second-key"));
        providers.insert("third".to_string(), ok("plain-third-key"));
        providers.insert("broken".to_string(), ok("env:CHAIN_MISSING_KEY"));
        let cfg = AppConfig {
            providers,
            fallback_chain: vec![
                "primary".into(),
                "broken".into(),
                "ghost".into(),
                "third".into(),
                "second".into(),
            ],
            ..Default::default()
        };

        let chain = build_fallback_chain(&cfg, "primary", None, None);
        let names: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["third", "second"]);
    }

    #[test]
    fn test_model_info_with_devin_swe2() {
        // `spawn()`/boot resolution reads the global `default_preferences`
        // row, which is machine-local developer state — a dev box saved with
        // thinking=max made the bare assertions read ambient settings instead
        // of the test's own. Pin the winning layer *before* boot: per-workspace
        // prefs outrank global defaults, and the session actor's slots are
        // seeded from them at spawn (`mirror_active_settings` then reports
        // those slots, so writing prefs post-spawn would be too late).
        let dir = tempfile::tempdir().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let store = agent_store::SessionStore::open(&canon).unwrap();
        store
            .save_prefs(&agent_store::WorkspacePrefs {
                provider: Some("devin".into()),
                model: Some("devin/swe-2".into()),
                thinking_level: Some("medium".into()),
                permission_mode: Some("default".into()),
                ..Default::default()
            })
            .unwrap();

        let (mut mgr, _rx) =
            SessionManager::spawn_at(Some(canon.clone()), test_paths_with_config());

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

        // The store wrote rows under the temp workspace's key — drop them so
        // the shared db isn't left with a dead workspace.
        store.delete_workspace().unwrap();
    }

    #[test]
    fn model_info_clamps_restored_and_stale_thinking_levels() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let paths = AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(
            paths.config_dir.join("config.toml"),
            r#"
active_provider = "test"
active_model = "a"
[providers.test]
type = "openai_compat"
base_url = "https://example.invalid/v1"
api_key = "test"
[[providers.test.models]]
id = "a"
reasoning = true
[providers.test.models.thinking_level_map]
low = "low"
high = "high"
xhigh = "xhigh"
[providers.test.model_overrides.a.thinking_level_map]
xhigh = ""
max = "max"
"#,
        )
        .unwrap();
        let store = agent_store::SessionStore::open(&root).unwrap();
        store
            .save_prefs(&agent_store::WorkspacePrefs {
                provider: Some("test".into()),
                model: Some("a".into()),
                thinking_level: Some("xhigh".into()),
                ..Default::default()
            })
            .unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(root), paths);
        let info = mgr.model_info();
        let active = info
            .models
            .iter()
            .find(|model| model.provider == "test" && model.model == "a")
            .unwrap();
        assert_eq!(active.available_levels, ["off", "low", "high", "max"]);
        assert_eq!(info.active_thinking_level.as_deref(), Some("low"));
        assert_eq!(
            mgr.active()
                .unwrap()
                .thinking_level
                .read()
                .unwrap()
                .as_deref(),
            Some("low")
        );
        // Model-info can run between a config reload and its actor command.
        *mgr.active().unwrap().thinking_level.write().unwrap() = Some("xhigh".into());
        assert_eq!(
            mgr.model_info().active_thinking_level.as_deref(),
            Some("low")
        );
        store.delete_workspace().unwrap();
    }

    #[test]
    fn test_workspace_prefs_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let (mut mgr, _rx) =
                SessionManager::spawn_at(Some(path.clone()), test_paths_with_config());
            mgr.set_model("devin".into(), "devin/swe-2".into());
            mgr.set_thinking_level("high".into());
            mgr.set_permission_mode("acceptEdits".into());
        }
        let (mut mgr2, _rx) = SessionManager::spawn_at(Some(path), test_paths());
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
        /// A stand-in actor — only its agent-mode slot is read here.
        fn fake_handle(
            id: i64,
            mode: String,
        ) -> (
            SessionHandle,
            tokio::sync::mpsc::Receiver<crate::command::UiCommand>,
        ) {
            let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(4);
            let (ui, _ui_rx) = agent_kernel::channels::UiSink::channel();
            (
                SessionHandle {
                    id,
                    cmd_tx,
                    decision: Arc::new(Mutex::new(None)),
                    pending_approval: Default::default(),
                    ask: Arc::new(agent_kernel::tools::registry::AskChannel::new(None)),
                    permissions: Arc::new(std::sync::RwLock::new(
                        agent_kernel::permissions::PermissionGate::from_mode_str("default"),
                    )),
                    agent_mode: Arc::new(std::sync::RwLock::new(mode)),
                    thinking_level: Arc::new(std::sync::RwLock::new(None)),
                    model: Arc::default(),
                    steer_tx: tokio::sync::mpsc::channel(1).0,
                    queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                    cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    shutdown: Arc::new(tokio::sync::Notify::new()),
                    preview: String::new(),
                    linked_roots: Default::default(),
                    notices: tokio::sync::mpsc::channel(1).0,
                    running: false,
                    ui,
                    bg_jobs: Default::default(),
                    bg_restored: Default::default(),
                },
                cmd_rx,
            )
        }

        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), test_paths());
        mgr.workspace_active = true;

        // Receivers stay alive: a dropped one closes the queue the config
        // reload pushes `ReloadModel` into.
        let mut _live = Vec::new();
        for (id, mode) in [(1, "plan".to_string()), (2, "build".to_string())] {
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
        use agent_store::SessionMeta;
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), test_paths());
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
                ..Default::default()
            };
            mgr.store.upsert_meta(meta).unwrap();
            let (cmd_tx, _rx) = tokio::sync::mpsc::channel(4);
            let (ui, _ui_rx) = agent_kernel::channels::UiSink::channel();
            mgr.handles.insert(
                id,
                SessionHandle {
                    id,
                    cmd_tx,
                    decision: Arc::new(Mutex::new(None)),
                    pending_approval: Default::default(),
                    ask: Arc::new(agent_kernel::tools::registry::AskChannel::new(None)),
                    permissions: Arc::new(std::sync::RwLock::new(
                        agent_kernel::permissions::PermissionGate::from_mode_str("default"),
                    )),
                    agent_mode: Arc::new(std::sync::RwLock::new("build".to_string())),
                    thinking_level: Arc::new(std::sync::RwLock::new(None)),
                    model: Arc::default(),
                    steer_tx: tokio::sync::mpsc::channel(1).0,
                    queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                    cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    shutdown: Arc::new(tokio::sync::Notify::new()),
                    preview: String::new(),
                    linked_roots: Default::default(),
                    notices: tokio::sync::mpsc::channel(1).0,
                    running: false,
                    ui,
                    bg_jobs: Default::default(),
                    bg_restored: Default::default(),
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

        // A composer pick lands in the live slot before the actor has drained
        // `SetModel` (it waits behind a running turn), so the meta row still
        // names the old model. `model_info` must already report the pick —
        // reading the meta first is what snapped the picker back for a whole turn.
        *mgr.handles[&2].model.write().unwrap() = ("openai".into(), "gpt-5".into());
        let info = mgr.model_info();
        assert_eq!(info.active_provider, "openai");
        assert_eq!(info.active_model, "gpt-5");
    }

    /// A session that never picked its own settings falls back to the
    /// workspace default on activation — per-session fields are `None` until
    /// the session itself chooses.
    #[test]
    fn a_session_without_settings_falls_back_to_the_workspace_default() {
        use agent_store::SessionMeta;
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) =
            SessionManager::spawn_at(Some(dir.path().to_path_buf()), test_paths_with_config());
        mgr.workspace_active = true;
        // The workspace default the fresh session should inherit — seeded in
        // the store, where the resolver reads it. Not on the manager's mirror:
        // `open_session` re-resolves the default before using it as a
        // fallback, so a hand-set mirror never acts as one.
        mgr.store
            .save_prefs(&agent_store::WorkspacePrefs {
                provider: Some("devin".into()),
                model: Some("devin/swe-2".into()),
                ..Default::default()
            })
            .unwrap();
        // A stale mirror left by the previously active session must NOT be the
        // fallback — that is the "changed the model in one session and another
        // session changed too" leak.
        mgr.provider_name = "stale-p".into();
        mgr.model_name = "stale-m".into();

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
            ..Default::default()
        };
        mgr.store.upsert_meta(meta).unwrap();
        mgr.metas = mgr.store.list();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel(4);
        let (ui, _ui_rx) = agent_kernel::channels::UiSink::channel();
        mgr.handles.insert(
            7,
            SessionHandle {
                id: 7,
                cmd_tx,
                decision: Arc::new(Mutex::new(None)),
                pending_approval: Default::default(),
                ask: Arc::new(agent_kernel::tools::registry::AskChannel::new(None)),
                permissions: Arc::new(std::sync::RwLock::new(
                    agent_kernel::permissions::PermissionGate::from_mode_str("auto"),
                )),
                agent_mode: Arc::new(std::sync::RwLock::new("plan".to_string())),
                thinking_level: Arc::new(std::sync::RwLock::new(None)),
                model: Arc::default(),
                steer_tx: tokio::sync::mpsc::channel(1).0,
                queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                shutdown: Arc::new(tokio::sync::Notify::new()),
                preview: String::new(),
                running: false,
                ui,
                bg_jobs: Default::default(),
                bg_restored: Default::default(),
                linked_roots: Default::default(),
                notices: tokio::sync::mpsc::channel(1).0,
            },
        );

        mgr.open_session(7);
        // Model/provider fall back to the workspace default (meta unset), not
        // to the stale mirror; the live handle's mode/level slots still
        // override where they exist.
        assert_eq!(mgr.provider_name, "devin");
        assert_eq!(mgr.model_name, "devin/swe-2");
        assert_eq!(mgr.permission_mode, "auto");
        assert_eq!(mgr.agent_mode, "plan");
    }

    /// Model selection layers: workspace pair > Agent 偏好 default >
    /// config fallback, with a deleted provider falling through.
    #[test]
    fn resolve_model_pair_layers_workspace_then_settings_then_config() {
        let providers: std::collections::HashMap<String, agent_llm::ProviderConfig> =
            serde_json::from_value(serde_json::json!({
                "a": {"kind": "openai_completions", "base_url": "https://x"},
                "b": {"kind": "openai_completions", "base_url": "https://y"},
            }))
            .unwrap();
        let defaults: crate::prefs::DefaultPreferences =
            serde_json::from_value(serde_json::json!({
                "active_provider": "b",
                "active_model": "b-model",
            }))
            .unwrap();
        let fb = || ("cfg-p".to_string(), "cfg-m".to_string());

        // Workspace pair wins over everything.
        let got = resolve_model_pair(
            &Some("a".into()),
            &Some("a-model".into()),
            Some(&defaults),
            &providers,
            fb(),
        );
        assert_eq!(got, ("a".to_string(), "a-model".to_string()));

        // No workspace pair → the settings-pane default.
        let got = resolve_model_pair(&None, &None, Some(&defaults), &providers, fb());
        assert_eq!(got, ("b".to_string(), "b-model".to_string()));

        // A default pair pointing at a deleted provider falls to config.
        let stale: crate::prefs::DefaultPreferences = serde_json::from_value(serde_json::json!({
            "active_provider": "gone",
            "active_model": "m",
        }))
        .unwrap();
        let got = resolve_model_pair(&None, &None, Some(&stale), &providers, fb());
        assert_eq!(got, ("cfg-p".to_string(), "cfg-m".to_string()));

        // No stored default at all → config.
        let got = resolve_model_pair(&None, &None, None, &providers, fb());
        assert_eq!(got, ("cfg-p".to_string(), "cfg-m".to_string()));
    }

    /// Office mode is workspace-scoped: the office pseudo-workspace is
    /// pinned to `office` regardless of stored prefs, and a stored `office`
    /// value on a regular project degrades to `build` — the mode switch
    /// lives in the sidebar, not the per-session picker.
    #[test]
    fn office_workspace_pins_agent_mode_to_office() {
        let office = crate::office::office_root().unwrap();
        assert_eq!(resolve_agent_mode(&office, &None, None), "office");

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_agent_mode(dir.path(), &Some("office".to_string()), None),
            "build"
        );
        assert_eq!(
            resolve_agent_mode(dir.path(), &Some("plan".to_string()), None),
            "plan"
        );
    }

    /// Office carries no workspace preference layer. Every office session
    /// shares one root, so a model pinned there by an earlier office session
    /// used to outrank the Agent 偏好 default — the next new office session
    /// silently kept the stale model instead of the configured default.
    #[test]
    fn office_workspace_ignores_a_pinned_model() {
        let pinned = agent_store::WorkspacePrefs {
            provider: Some("a".into()),
            model: Some("a-model".into()),
            thinking_level: Some("max".into()),
            permission_mode: Some("bypassPermissions".into()),
            agent_mode: Some("plan".into()),
            ..Default::default()
        };

        // A real project keeps its pin.
        let dir = tempfile::tempdir().unwrap();
        let kept = workspace_layer(dir.path(), &pinned);
        assert_eq!(kept.provider.as_deref(), Some("a"));
        assert_eq!(kept.model.as_deref(), Some("a-model"));
        assert_eq!(kept.thinking_level.as_deref(), Some("max"));

        // Office drops it entirely, so the Agent 偏好 defaults layer wins.
        let office = crate::office::office_root().unwrap();
        let dropped = workspace_layer(&office, &pinned);
        assert_eq!(dropped.provider, None);
        assert_eq!(dropped.model, None);
        assert_eq!(dropped.thinking_level, None);
        assert_eq!(dropped.permission_mode, None);
        assert_eq!(dropped.agent_mode, None);
    }

    /// Saving a default must not touch the session in front, and saving an
    /// unrelated row must not overwrite the saved defaults with the active
    /// session's mirror — the "I set 自动 and it keeps wandering off" bug.
    #[test]
    fn default_prefs_are_independent_of_the_active_session() {
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), test_paths());
        let saved = crate::prefs::try_load_default_preferences();
        // A workspace pin the new default has to displace.
        mgr.store
            .save_prefs(&agent_store::WorkspacePrefs {
                permission_mode: Some("bypassPermissions".into()),
                model: Some("kept".into()),
                ..Default::default()
            })
            .unwrap();
        // The active session's mirror.
        mgr.permission_mode = "bypassPermissions".into();
        mgr.agent_mode = "plan".into();

        mgr.set_default_prefs(Some("acceptEdits".into()), None, None, None, None, None);
        // The live display is the session's, not the new default.
        assert_eq!(mgr.permission_mode, "bypassPermissions");
        assert_eq!(mgr.agent_mode, "plan");
        let stored = crate::prefs::try_load_default_preferences().unwrap();
        assert_eq!(stored.permission_mode, "acceptEdits");
        // The re-defaulted field's pin is dropped; untouched pins stay.
        let ws = mgr.store.load_prefs();
        assert_eq!(ws.permission_mode, None);
        assert_eq!(ws.model.as_deref(), Some("kept"));

        // An unrelated save keeps the stored default, not the mirror.
        mgr.set_memory_prefs(None, None);
        let stored = crate::prefs::try_load_default_preferences().unwrap();
        assert_eq!(stored.permission_mode, "acceptEdits");
        assert_ne!(stored.agent_mode, "plan");
        assert_eq!(mgr.default_prefs().0, "acceptEdits");

        if let Some(p) = saved {
            let _ = crate::prefs::save_default_preferences(&p);
        }
        mgr.store.delete_workspace().unwrap();
    }

    /// A composer change pins only the field it changed — picking a model
    /// must not freeze the mirrored permission / mode / level into the
    /// workspace, which would re-pin what `set_default_prefs` just cleared.
    #[test]
    fn composer_setters_pin_only_their_own_field() {
        let dir = tempfile::tempdir().unwrap();
        let (mut mgr, _rx) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), test_paths());
        mgr.store
            .save_prefs(&agent_store::WorkspacePrefs::default())
            .unwrap();
        mgr.permission_mode = "bypassPermissions".into();
        mgr.active_thinking_level = Some("max".into());

        mgr.set_model("p".into(), "m".into());
        let ws = mgr.store.load_prefs();
        assert_eq!(ws.provider.as_deref(), Some("p"));
        assert_eq!(ws.model.as_deref(), Some("m"));
        assert_eq!(ws.permission_mode, None);
        assert_eq!(ws.thinking_level, None);
        assert_eq!(ws.agent_mode, None);

        mgr.set_thinking_level("high".into());
        let ws = mgr.store.load_prefs();
        assert_eq!(ws.thinking_level.as_deref(), Some("high"));
        assert_eq!(ws.model.as_deref(), Some("m"));
        assert_eq!(ws.permission_mode, None);
        mgr.store.delete_workspace().unwrap();
    }

    /// The export keeps the conversation and drops what the model saw but
    /// the user never did (the system prompt, hidden nudges).
    #[test]
    fn markdown_export_renders_turns_and_skips_hidden_rows() {
        use agent_llm::types::{ChatMessage, NoticeKind, Role};
        let mut hidden = ChatMessage::user("nudge");
        hidden.notice = Some(NoticeKind::Hidden);
        let mut tool = ChatMessage::assistant("");
        tool.tool_calls = Some(vec![agent_llm::types::ToolCall {
            id: "c1".into(),
            name: "bash".into(),
            arguments: "{\"cmd\":\"ls\"}".into(),
        }]);
        let mut result = ChatMessage::assistant("a.txt");
        result.role = Role::Tool;
        let md = session_markdown(
            "标题",
            3,
            &[
                ChatMessage::system("SYSTEM PROMPT"),
                ChatMessage::user("你好"),
                hidden,
                tool,
                result,
                ChatMessage::assistant("完成"),
            ],
        );
        assert!(md.starts_with("# 标题\n\n> 会话 ID: 3"));
        assert!(md.contains("## 用户\n\n你好"));
        assert!(md.contains("`bash`") && md.contains("{\"cmd\":\"ls\"}"));
        assert!(md.contains("a.txt"));
        assert!(md.contains("## 助手\n\n完成"));
        assert!(!md.contains("SYSTEM PROMPT") && !md.contains("nudge"));
    }

    /// The「默认进入」preference picks the boot workspace: 工作 opens the
    /// office pseudo-workspace, 编程 the most recent real project (office
    /// skipped — boot records it as the newest recent, so taking the head
    /// verbatim would ignore the setting), 上次使用 the MRU head as-is.
    #[test]
    fn default_workspace_mode_picks_the_boot_root() {
        let recent = |path: &std::path::Path| crate::prefs::RecentWorkspace {
            path: path.to_path_buf(),
            name: "p".into(),
            last_opened: 0,
        };
        let office = crate::office::office_root().unwrap();
        // The real app creates this lazily on the first office switch; the
        // canonicalising boot path needs it to exist to keep the entry.
        std::fs::create_dir_all(&office).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir(&project).unwrap();
        // MRU head is the office workspace — what a previous 工作 boot leaves.
        let recents = vec![recent(&office), recent(&project)];

        let prefs = |mode: &str| -> crate::prefs::DefaultPreferences {
            serde_json::from_value(serde_json::json!({ "default_workspace_mode": mode })).unwrap()
        };

        // 工作 boots into office — recents don't matter.
        assert_eq!(
            boot_workspace(Some(&prefs("office")), &[]),
            Some(office.clone())
        );
        // 编程 skips the office head for the real project below it.
        assert_eq!(
            boot_workspace(Some(&prefs("build")), &recents),
            Some(project.clone())
        );
        // 上次使用 (default) resumes whatever was open — office included.
        assert_eq!(
            boot_workspace(Some(&prefs("last")), &recents),
            Some(office.clone())
        );
        // An untouched install stores nothing: same as 上次使用.
        assert_eq!(boot_workspace(None, &recents), Some(office));
        // Nothing to open → empty state.
        assert_eq!(boot_workspace(None, &[]), None);
        assert_eq!(boot_workspace(Some(&prefs("build")), &[]), None);
    }

    /// The engine empties its slot when it takes a verdict; delivery reports
    /// success as soon as it sees that, without waiting out the pickup window.
    #[test]
    fn a_verdict_the_engine_takes_is_delivered() {
        let slot: Arc<Mutex<Option<(u64, agent_kernel::tools::registry::ToolVerdict)>>> =
            Arc::new(Mutex::new(None));
        let engine = {
            let slot = slot.clone();
            std::thread::spawn(move || loop {
                let mut s = slot.lock().unwrap();
                if s.as_ref().is_some_and(|(id, v)| *id == 7 && v.approved) {
                    *s = None;
                    return;
                }
                drop(s);
                std::thread::sleep(std::time::Duration::from_millis(50));
            })
        };
        let started = std::time::Instant::now();
        assert_eq!(
            deliver_decision(&slot, 7, true, None),
            Ok::<(), String>(())
        );
        assert!(started.elapsed() < DECISION_PICKUP);
        engine.join().unwrap();
    }

    /// Nothing waiting: the verdict is withdrawn, so it cannot approve a later
    /// request that happens to reuse the slot, and the caller hears about it.
    #[test]
    fn a_verdict_nobody_takes_is_withdrawn_and_reported() {
        let slot = Mutex::new(None);
        assert!(deliver_decision(&slot, 3, true, None).is_err());
        assert!(slot.lock().unwrap().is_none());
    }

    /// `build` reads the rows from the store and overlays the live actor state
    /// the snapshot carried out of the manager.
    #[test]
    fn the_projects_snapshot_builds_rows_with_the_live_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap());
        let mut meta = SessionMeta {
            id: 1,
            title: "t".into(),
            ..Default::default()
        };
        meta.updated_at = 1;
        store.upsert_meta(meta).unwrap();
        let snapshot = ProjectsSnapshot {
            projects: vec![SnapshotProject {
                root: "r".into(),
                name: "r".into(),
                last_opened: 0,
                current: true,
                office: false,
                store: StoreRef::Open(store),
                live: HashMap::from([(1, ("live".to_string(), true))]),
                active_id: 1,
            }],
        };
        let tree = snapshot.build();
        let row = &tree[0].sessions[0];
        assert_eq!(row.preview, "live");
        assert!(row.running && row.active);
    }

    /// A record from a previous run cannot claim a job is still going: the
    /// process that ran it is gone, so the sidebar shows it as interrupted.
    #[test]
    fn a_saved_running_job_reads_as_interrupted() {
        let saved = serde_json::json!([
            {"id": "bg-1", "command": "sleep 30", "state": {"kind": "running"}},
            {"id": "bg-2", "command": "echo hi", "state": {"kind": "paused"}},
            {"id": "bg-3", "command": "true",
             "state": {"kind": "done", "label": "exited(0)", "code": 0, "reason": "exited"}},
        ]);
        let shown = crate::session::restored_bg_jobs(saved);
        assert_eq!(shown[0]["state"]["kind"], "done");
        assert_eq!(shown[0]["state"]["label"], "应用已退出");
        assert_eq!(shown[0]["state"]["reason"], "error");
        assert_eq!(shown[1]["state"]["kind"], "done");
        // An ending that happened before the restart is left alone.
        assert_eq!(shown[2]["state"]["label"], "exited(0)");
        assert_eq!(shown[2]["state"]["reason"], "exited");
        // Nothing behind these entries has a process: the pane must not offer
        // controls or a log tail it cannot honour.
        for job in &shown {
            assert_eq!(job["restored"], true, "{job}");
        }
    }

    /// A restored id cannot collide with the new process's `bg-1`, and an entry
    /// restored once keeps its id through every later restart — or the pane's
    /// remove would hit the wrong row.
    #[test]
    fn restored_ids_are_unique_and_stable() {
        let saved = serde_json::json!([
            {"id": "bg-1", "command": "a", "started_at_ms": 7, "state": {"kind": "running"}},
        ]);
        let once = crate::session::restored_bg_jobs(saved);
        assert_eq!(once[0]["id"], "bg-1@7");
        let twice = crate::session::restored_bg_jobs(serde_json::Value::Array(once.clone()));
        assert_eq!(twice, once);
    }

    /// The record keeps the most recent entries only.
    #[test]
    fn a_restored_record_is_capped_to_the_newest() {
        let saved: Vec<serde_json::Value> = (0..80)
            .map(|i| serde_json::json!({"id": format!("bg-{i}"), "started_at_ms": i}))
            .collect();
        let kept = crate::session::restored_bg_jobs(serde_json::Value::Array(saved));
        assert_eq!(kept.len(), 50);
        assert_eq!(kept.last().unwrap()["id"], "bg-79@79");
    }
}
