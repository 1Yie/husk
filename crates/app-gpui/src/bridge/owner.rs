use super::*;
use agent_ipc::{AgentState, UiEvent};
use agent_kernel::channels::UiSink;
use agent_kernel::permissions::{PermissionGate, PermissionMode, PermissionRules};
use agent_kernel::tools::registry::{AskChannel, ToolVerdict};
use husk_host::command::UiCommand;
use husk_host::session_manager::{SessionHandle, SessionManager};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::Duration;

static OWNER_ACTIVE: AtomicBool = AtomicBool::new(false);
// The host's repository is process-global and cannot be rebound after boot.
static OWNER_PATHS: Mutex<Option<agent_paths::AppPaths>> = Mutex::new(None);

type DecisionSlot = Arc<Mutex<Option<(u64, ToolVerdict)>>>;
type ApprovalId = (SessionKey, u64, Option<u64>);

#[derive(Clone)]
struct Endpoint {
    cmd: tokio::sync::mpsc::Sender<UiCommand>,
    decision: DecisionSlot,
    pending: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    shutdown: Arc<tokio::sync::Notify>,
    permissions: Arc<RwLock<PermissionGate>>,
    ask: Arc<AskChannel>,
    ui: UiSink,
    queue: Arc<Mutex<VecDeque<String>>>,
}

impl Endpoint {
    fn of(handle: &SessionHandle) -> Result<Self, String> {
        let endpoint = Self {
            cmd: handle.cmd_tx.clone(),
            decision: handle.decision.clone(),
            pending: handle.pending_approval.clone(),
            cancel: handle.cancel.clone(),
            shutdown: handle.shutdown.clone(),
            permissions: handle.permissions.clone(),
            ask: handle.ask.clone(),
            ui: handle.ui.clone(),
            queue: handle.queue.clone(),
        };
        endpoint
            .permissions
            .write()
            .map_err(|e| e.to_string())?
            .merge_rules(PermissionRules {
                deny: ["computer".to_string()].into_iter().collect(),
                ..Default::default()
            });
        Ok(endpoint)
    }
}

#[derive(Clone, Default)]
struct Live {
    running: Option<bool>,
    preview: Option<String>,
}

#[derive(Default)]
struct Routing {
    settings: Option<serde_json::Value>,
    endpoints: HashMap<SessionKey, Endpoint>,
    live: HashMap<SessionKey, Live>,
    approvals: HashMap<ApprovalId, PendingApproval>,
    decisions_in_flight: HashSet<ApprovalId>,
    history_loaded: HashSet<SessionKey>,
    baselines: HashMap<SessionKey, Vec<agent_llm::types::ChatMessage>>,
    sequence: u64,
}

impl Routing {
    fn observe(&mut self, key: &SessionKey, event: &UiEvent) {
        let live = self.live.entry(key.clone()).or_default();
        match event {
            UiEvent::StateChanged(state) => {
                live.running = Some(state.is_active());
                if !matches!(state, AgentState::AwaitingToolConfirmation { .. }) {
                    if let Some(endpoint) = self.endpoints.get(key) {
                        endpoint.pending.store(0, Ordering::Release);
                    }
                    self.approvals.retain(|(owner, _, child), _| {
                        owner != key || (state.is_active() && child.is_some())
                    });
                    self.decisions_in_flight
                        .retain(|id| self.approvals.contains_key(id));
                }
            }
            UiEvent::AssistantMessage(text) => {
                live.preview = Some(text.chars().take(60).collect());
            }
            UiEvent::ApprovalRequested {
                request_id,
                tool_name,
                diff,
                fuzzy,
                child_id,
                actions,
            } => {
                if child_id.is_none()
                    && let Some(endpoint) = self.endpoints.get(key)
                {
                    endpoint.pending.store(*request_id, Ordering::Release);
                }
                self.approvals.insert(
                    (key.clone(), *request_id, *child_id),
                    PendingApproval {
                        key: key.clone(),
                        request_id: *request_id,
                        tool_name: tool_name.clone(),
                        diff: diff.clone(),
                        fuzzy: *fuzzy,
                        child_id: *child_id,
                        actions: actions.clone(),
                    },
                );
            }
            UiEvent::ApprovalTimedOut {
                request_id,
                child_id,
            } => {
                let id = (key.clone(), *request_id, *child_id);
                self.approvals.remove(&id);
                self.decisions_in_flight.remove(&id);
                if child_id.is_none()
                    && let Some(endpoint) = self.endpoints.get(key)
                {
                    let _ = endpoint.pending.compare_exchange(
                        *request_id,
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            }
            UiEvent::ToolCallFinished {
                child_id: Some(child),
                ..
            } => {
                self.approvals
                    .retain(|(owner, _, id), _| owner != key || *id != Some(*child));
                self.decisions_in_flight
                    .retain(|id| self.approvals.contains_key(id));
            }
            _ => {}
        }
    }
}

enum Work {
    Command(u64, Command),
    Shutdown,
}

pub(super) struct Client {
    commands: mpsc::Sender<Work>,
    controls: mpsc::Sender<(u64, Command)>,
    events: async_channel::Receiver<BridgeEvent>,
    routing: Arc<Mutex<Routing>>,
    stopped: Arc<AtomicBool>,
    next: AtomicU64,
}

fn error(events: &async_channel::Sender<BridgeEvent>, request_id: u64, message: impl Into<String>) {
    let _ = events.try_send(BridgeEvent::Error {
        request_id,
        message: message.into(),
    });
}

impl Client {
    pub(super) fn spawn(paths: agent_paths::AppPaths) -> io::Result<Self> {
        if OWNER_ACTIVE.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a host bridge is already running",
            ));
        }
        {
            let mut bound = OWNER_PATHS.lock().unwrap_or_else(|e| e.into_inner());
            if bound.as_ref().is_some_and(|previous| previous != &paths) {
                OWNER_ACTIVE.store(false, Ordering::Release);
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "the host repository is already bound to different application paths",
                ));
            }
            *bound = Some(paths.clone());
        }
        let (commands, rx) = mpsc::channel();
        let (controls, control_rx) = mpsc::channel();
        let (events_tx, events) = async_channel::unbounded();
        let routing = Arc::new(Mutex::new(Routing::default()));
        let stopped = Arc::new(AtomicBool::new(false));
        let control_routing = routing.clone();
        let control_events = events_tx.clone();
        let control_stop = stopped.clone();
        if let Err(e) = thread::Builder::new()
            .name("gpui-host-control".into())
            .spawn(move || {
                while !control_stop.load(Ordering::Acquire) {
                    match control_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok((id, command)) => control(
                            id,
                            command,
                            &control_routing,
                            &control_events,
                            &control_stop,
                        ),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
        {
            OWNER_ACTIVE.store(false, Ordering::Release);
            return Err(e);
        }
        let owner_routing = routing.clone();
        let owner_stop = stopped.clone();
        let spawn = thread::Builder::new()
            .name("gpui-host-owner".into())
            .spawn(move || {
                struct Guard;
                impl Drop for Guard {
                    fn drop(&mut self) {
                        OWNER_ACTIVE.store(false, Ordering::Release);
                    }
                }
                let _guard = Guard;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(
                        paths,
                        rx,
                        owner_routing.clone(),
                        owner_stop.clone(),
                        events_tx.clone(),
                    )
                }));
                if let Err(panic) = outcome {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "host owner panicked".into());
                    error(&events_tx, 0, message);
                }
                owner_stop.store(true, Ordering::Release);
                stop_endpoints(&owner_routing);
                events_tx.close();
            });
        if let Err(e) = spawn {
            stopped.store(true, Ordering::Release);
            OWNER_ACTIVE.store(false, Ordering::Release);
            return Err(e);
        }
        Ok(Self {
            commands,
            controls,
            events,
            routing,
            stopped,
            next: AtomicU64::new(1),
        })
    }

    pub(super) fn events(&self) -> async_channel::Receiver<BridgeEvent> {
        self.events.clone()
    }

    pub(super) fn send(&self, command: Command) -> Result<u64, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("host bridge is shut down".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if let Command::Cancel { key } = &command {
            let endpoint = endpoint(&self.routing, key)?;
            endpoint.cancel.store(true, Ordering::Release);
            return Ok(id);
        }
        if matches!(
            command,
            Command::Decide { .. }
                | Command::RevokeSessionTool { .. }
                | Command::AnswerQuestion { .. }
        ) {
            self.controls
                .send((id, command))
                .map_err(|e| e.to_string())?;
        } else {
            self.commands
                .send(Work::Command(id, command))
                .map_err(|e| e.to_string())?;
        }
        Ok(id)
    }

    pub(super) fn shutdown(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            stop_endpoints(&self.routing);
            let _ = self.commands.send(Work::Shutdown);
        }
    }
}

fn endpoint(routing: &Arc<Mutex<Routing>>, key: &SessionKey) -> Result<Endpoint, String> {
    routing
        .lock()
        .map_err(|e| e.to_string())?
        .endpoints
        .get(key)
        .cloned()
        .ok_or_else(|| "session is no longer live".into())
}

fn stop_endpoints(routing: &Arc<Mutex<Routing>>) {
    let endpoints: Vec<_> = routing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .endpoints
        .values()
        .cloned()
        .collect();
    for endpoint in endpoints {
        endpoint.cancel.store(true, Ordering::Release);
        endpoint.shutdown.notify_one();
    }
}

fn control(
    id: u64,
    command: Command,
    routing: &Arc<Mutex<Routing>>,
    events: &async_channel::Sender<BridgeEvent>,
    stopped: &Arc<AtomicBool>,
) {
    let result = (|| -> Result<(), String> {
        match command {
            Command::Decide {
                key,
                request_id,
                approved,
                action,
                child_id,
            } => {
                let approval_id = (key.clone(), request_id, child_id);
                let endpoint = {
                    let mut state = routing.lock().map_err(|e| e.to_string())?;
                    let approval = state.approvals.get(&approval_id).ok_or_else(|| {
                        format!(
                            "approval #{request_id} is stale or belongs to another session/child"
                        )
                    })?;
                    if approved && approval.tool_name == "computer" {
                        return Err("Computer control is disabled until the native safety HUD is verified; reject or cancel this request.".into());
                    }
                    // Linking grants require the host's path-policy validation, not a plain verdict.
                    if approved && action.is_some() {
                        return Err("This approval action requires a workspace grant interface that is not available.".into());
                    }
                    if state.decisions_in_flight.contains(&approval_id) {
                        return Err(format!("approval #{request_id} is already being delivered"));
                    }
                    let endpoint = state
                        .endpoints
                        .get(&key)
                        .cloned()
                        .ok_or_else(|| "approval session is no longer live".to_string())?;
                    state.decisions_in_flight.insert(approval_id.clone());
                    endpoint
                };
                let routing_bg = routing.clone();
                let events_bg = events.clone();
                let stop_bg = stopped.clone();
                let spawn = thread::Builder::new()
                    .name("gpui-host-decision".into())
                    .spawn(move || {
                        let delivered = if stop_bg.load(Ordering::Acquire) {
                            Err("host bridge is shut down".into())
                        } else if child_id.is_some() {
                            if agent_kernel::tools::delegate::deliver_child_decision(
                                request_id, approved, action,
                            ) {
                                Ok(())
                            } else {
                                Err(format!("child approval #{request_id} is no longer pending"))
                            }
                        } else {
                            husk_host::session_manager::deliver_decision(
                                &endpoint.decision,
                                request_id,
                                approved,
                                action,
                            )
                        };
                        let mut state = routing_bg.lock().unwrap_or_else(|e| e.into_inner());
                        state.decisions_in_flight.remove(&approval_id);
                        if delivered.is_ok() {
                            state.approvals.remove(&approval_id);
                        }
                        drop(state);
                        if let Err(message) = delivered {
                            error(&events_bg, id, message);
                        }
                    });
                if let Err(e) = spawn {
                    routing
                        .lock()
                        .map_err(|e| e.to_string())?
                        .decisions_in_flight
                        .remove(&(key, request_id, child_id));
                    return Err(e.to_string());
                }
            }
            Command::RevokeSessionTool { key, tool_name } => {
                let endpoint = endpoint(routing, &key)?;
                endpoint
                    .permissions
                    .write()
                    .map_err(|e| e.to_string())?
                    .revoke_grants(&tool_name);
                endpoint
                    .ui
                    .send(UiEvent::SessionToolWhitelisted {
                        tool_name,
                        stopped: true,
                    })
                    .map_err(|e| format!("{e:?}"))?;
            }
            Command::AnswerQuestion {
                key,
                request_id,
                answer,
            } => {
                let endpoint = endpoint(routing, &key)?;
                if !endpoint.ask.answer(request_id, answer) {
                    return Err(format!("question #{request_id} is no longer pending"));
                }
                let _ = events.try_send(BridgeEvent::AnswerDelivered { request_id: id });
                endpoint
                    .ui
                    .send(UiEvent::QuestionAnswered { request_id })
                    .map_err(|e| format!("{e:?}"))?;
            }
            _ => return Err("invalid control command".into()),
        }
        Ok(())
    })();
    if let Err(message) = result {
        error(events, id, message);
    }
}

fn reconcile(manager: &mut SessionManager, routing: &Arc<Mutex<Routing>>) {
    let live: Vec<_> = routing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .live
        .iter()
        .map(|(key, live)| (key.clone(), live.clone()))
        .collect();
    for (key, live) in live {
        if let Some(handle) = manager.handle_mut_at(&key.root, key.id) {
            if let Some(running) = live.running {
                handle.running = running;
            }
            if let Some(preview) = live.preview {
                handle.preview = preview;
            }
        }
    }
}

fn register(manager: &SessionManager, routing: &Arc<Mutex<Routing>>) -> Result<(), String> {
    let root = manager.workspace_root.to_string_lossy().into_owned();
    let mut keys: HashSet<_> = routing
        .lock()
        .map_err(|e| e.to_string())?
        .endpoints
        .keys()
        .cloned()
        .collect();
    for (id, ..) in manager.sidebar_rows() {
        keys.insert(SessionKey {
            root: root.clone(),
            id,
        });
    }
    let mut endpoints = HashMap::new();
    for key in keys {
        if let Some(handle) = manager.handle_at(&key.root, key.id) {
            endpoints.insert(key, Endpoint::of(handle)?);
        }
    }
    let mut state = routing.lock().map_err(|e| e.to_string())?;
    for (key, endpoint) in &endpoints {
        let waiting = state
            .approvals
            .values()
            .find(|approval| &approval.key == key && approval.child_id.is_none())
            .map_or(0, |approval| approval.request_id);
        endpoint.pending.store(waiting, Ordering::Release);
    }
    state.endpoints = endpoints;
    let alive: HashSet<_> = state.endpoints.keys().cloned().collect();
    state.approvals.retain(|(key, _, _), _| alive.contains(key));
    state
        .decisions_in_flight
        .retain(|(key, _, _)| alive.contains(key));
    Ok(())
}

fn publish_snapshot(
    manager: &mut SessionManager,
    paths: &agent_paths::AppPaths,
    routing: &Arc<Mutex<Routing>>,
    events: &async_channel::Sender<BridgeEvent>,
    id: u64,
) -> Result<(), String> {
    reconcile(manager, routing);
    register(manager, routing)?;
    let active = manager.active().map(|handle| SessionKey {
        root: manager.workspace_root.to_string_lossy().into_owned(),
        id: handle.id,
    });
    let load_history = active.as_ref().is_some_and(|key| {
        !routing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history_loaded
            .contains(key)
    });
    // Only the first activation establishes a disk baseline. Later snapshots
    // must not replace live deltas with an older turn-boundary checkpoint.
    let history = if load_history {
        routing
            .lock()
            .map_err(|e| e.to_string())?
            .baselines
            .remove(active.as_ref().unwrap())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let projects = manager.projects_snapshot().build();
    let needs_settings = routing
        .lock()
        .map_err(|e| e.to_string())?
        .settings
        .as_ref()
        .is_none_or(|value| value["workspace"].as_str() != manager.workspace_root.to_str());
    if needs_settings {
        let settings = super::settings::snapshot(manager, paths);
        routing.lock().map_err(|e| e.to_string())?.settings = Some(settings);
    }
    let info = manager.model_info();
    let queued_prompts = manager
        .active()
        .map(|handle| {
            handle
                .queue
                .lock()
                .map(|queue| queue.iter().cloned().collect())
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .unwrap_or_default();
    let config = ConfigInfo {
        config_dir: paths.config_dir.clone(),
        data_dir: paths.data_dir.clone(),
        workspace_dir_name: paths.workspace_dir_name.clone(),
        permission_mode: info.active_permission_mode.clone().unwrap_or_default(),
        agent_mode: info.active_agent_mode.clone().unwrap_or_default(),
        thinking_level: info.active_thinking_level.clone(),
        theme_mode: husk_host::prefs::load_appearance_settings().theme_mode,
        desktop_notifications: husk_host::prefs::desktop_notifications_enabled().ok(),
        memory_preferences: husk_host::prefs::memory_preferences().ok(),
        computer_control_enabled: false,
    };
    let mut workspaces = Vec::new();
    let mut sessions = Vec::new();
    for project in projects {
        workspaces.push(WorkspaceRow {
            path: project.root.clone(),
            title: project.name,
            active: project.current,
        });
        for row in project.sessions.into_iter().filter(|row| !row.archived) {
            sessions.push(SessionRow {
                key: SessionKey {
                    root: project.root.clone(),
                    id: row.id,
                },
                title: row.title,
                preview: row.preview,
                running: row.running,
                pinned: row.pinned,
                pinned_global: row.pinned_global,
                updated_at: row.updated_at,
            });
        }
    }
    // Publication and event sequencing share only this memory lock, never I/O.
    let mut state = routing.lock().map_err(|e| e.to_string())?;
    for row in &mut sessions {
        if let Some(live) = state.live.get(&row.key) {
            if let Some(running) = live.running {
                row.running = running;
            }
            if let Some(preview) = &live.preview {
                row.preview = preview.clone();
            }
        }
    }
    if let Some(key) = active.as_ref().filter(|_| load_history) {
        state.history_loaded.insert(key.clone());
    }
    let mut model_info = serde_json::to_value(info).map_err(|e| e.to_string())?;
    if let Some(settings) = &state.settings {
        model_info["settings"] = settings.clone();
    }
    let snapshot = Snapshot {
        active,
        sessions,
        workspaces,
        history,
        queued_prompts,
        model_info,
        config,
        pending_approvals: state.approvals.values().cloned().collect(),
        history_loaded: load_history,
        event_sequence: state.sequence,
    };
    events
        .try_send(BridgeEvent::Snapshot {
            request_id: id,
            snapshot,
        })
        .map_err(|e| e.to_string())
}

fn run(
    paths: agent_paths::AppPaths,
    commands: mpsc::Receiver<Work>,
    routing: Arc<Mutex<Routing>>,
    stopped: Arc<AtomicBool>,
    events: async_channel::Sender<BridgeEvent>,
) {
    husk_host::office::init_store(paths.data_dir.clone());
    for recent in husk_host::prefs::load_recent_workspaces() {
        prepare_history(&recent.path, None, &routing);
    }
    if let Some(office) = husk_host::office::office_root() {
        prepare_history(&office, None, &routing);
    }
    let (mut manager, incoming) = SessionManager::spawn(paths.clone());
    if stopped.load(Ordering::Acquire) {
        let _ = register(&manager, &routing);
        stop_endpoints(&routing);
        return;
    }
    if let Err(message) = publish_snapshot(&mut manager, &paths, &routing, &events, 0) {
        error(&events, 0, message);
        return;
    }
    let event_routing = routing.clone();
    let event_stop = stopped.clone();
    let event_output = events.clone();
    let forwarder = thread::Builder::new()
        .name("gpui-host-events".into())
        .spawn(move || {
            while !event_stop.load(Ordering::Acquire) {
                match incoming.recv_timeout(Duration::from_millis(50)) {
                    Ok((root, id, event)) => {
                        let key = SessionKey { root, id };
                        let mut state = event_routing.lock().unwrap_or_else(|e| e.into_inner());
                        state.observe(&key, &event);
                        state.sequence = state.sequence.saturating_add(1);
                        if event_output
                            .try_send(BridgeEvent::Ui { key, event })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    if let Err(e) = forwarder {
        error(&events, 0, e.to_string());
        return;
    }
    while !stopped.load(Ordering::Acquire) {
        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(Work::Command(id, command)) => {
                reconcile(&mut manager, &routing);
                if let Command::Activity(command) = command {
                    match super::activity::handle(&mut manager, &paths, command) {
                        Ok((key, data)) => {
                            let _ = events.try_send(BridgeEvent::Activity {
                                request_id: id,
                                key,
                                data,
                            });
                        }
                        Err(message) => error(&events, id, message),
                    }
                    continue;
                }
                let submission = matches!(&command, Command::Send { .. });
                let queue_update = matches!(
                    &command,
                    Command::RemoveQueued { .. }
                        | Command::EditQueued { .. }
                        | Command::MoveQueued { .. }
                );
                if let Err(message) = execute(command, &mut manager, &paths, &routing) {
                    // A structural command may partially change manager state before failing.
                    let _ = register(&manager, &routing);
                    error(&events, id, message);
                    continue;
                }
                if submission {
                    let _ = events.try_send(BridgeEvent::SubmissionAccepted { request_id: id });
                }
                if queue_update {
                    let _ = events.try_send(BridgeEvent::QueueUpdated { request_id: id });
                }
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                if let Err(message) = publish_snapshot(&mut manager, &paths, &routing, &events, id)
                {
                    // Already-persisted changes must not be reported as failed
                    // submissions/edits merely because a later snapshot failed.
                    error(
                        &events,
                        if submission || queue_update { 0 } else { id },
                        message,
                    );
                }
            }
            Ok(Work::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    // Handles created by an operation overlapping shutdown must also be stopped.
    let _ = register(&manager, &routing);
    stop_endpoints(&routing);
}

fn active_command(manager: &SessionManager, command: UiCommand) -> Result<(), String> {
    let handle = manager
        .active()
        .ok_or_else(|| "no active session".to_string())?;
    handle.cmd_tx.try_send(command).map_err(|e| e.to_string())
}

fn require_active_session(manager: &SessionManager, key: &SessionKey) -> Result<(), String> {
    if manager.workspace_root.to_string_lossy() != key.root
        || manager.active().is_none_or(|handle| handle.id != key.id)
    {
        return Err("session is no longer active".into());
    }
    Ok(())
}

fn prepare_history(root: &std::path::Path, requested: Option<i64>, routing: &Arc<Mutex<Routing>>) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let Some(store) = agent_store::SessionStore::open_existing(&root) else {
        return;
    };
    let metas = store.list();
    let id = requested.or_else(|| {
        store
            .last_active()
            .filter(|id| metas.iter().any(|meta| meta.id == *id))
            .or_else(|| metas.iter().find(|meta| !meta.archived).map(|meta| meta.id))
    });
    let Some(id) = id else { return };
    let key = SessionKey {
        root: root.to_string_lossy().into_owned(),
        id,
    };
    {
        let state = routing.lock().unwrap_or_else(|e| e.into_inner());
        if state.history_loaded.contains(&key) || state.baselines.contains_key(&key) {
            return;
        }
    }
    // Actor events are all newer than this baseline, including events arriving
    // before the first snapshot. Never hold the routing lock across disk reads.
    let history = store.load_history(id).unwrap_or_default();
    routing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .baselines
        .insert(key, history);
}

fn require_known_session(manager: &SessionManager, key: &SessionKey) -> Result<(), String> {
    let projects = manager.projects_snapshot().build();
    if !projects.iter().any(|project| {
        project.root == key.root && project.sessions.iter().any(|row| row.id == key.id)
    }) {
        return Err("session does not exist in the specified workspace".into());
    }
    Ok(())
}

fn select_workspace(manager: &mut SessionManager, key: &SessionKey) -> Result<(), String> {
    require_known_session(manager, key)?;
    if manager.workspace_root.to_string_lossy() != key.root {
        manager
            .switch_workspace(PathBuf::from(&key.root))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn persist_queue(key: &SessionKey, items: Vec<String>) -> Result<(), String> {
    let store = agent_store::SessionStore::open_existing(std::path::Path::new(&key.root))
        .ok_or_else(|| "session storage is unavailable".to_string())?;
    store
        .update_queue(key.id, items)
        .map_err(|error| format!("could not save queued messages: {error}"))
}

fn enqueue(
    endpoint: &Endpoint,
    text: String,
    persist: impl FnOnce(Vec<String>) -> Result<(), String>,
) -> Result<(), String> {
    // Reserve the drain notification before modifying the deque. A full or
    // closed channel must leave the draft untouched and return an error.
    let permit = endpoint
        .cmd
        .try_reserve()
        .map_err(|error| error.to_string())?;
    let mut queue = endpoint.queue.lock().map_err(|error| error.to_string())?;
    queue.push_back(text);
    let items: Vec<_> = queue.iter().cloned().collect();
    if let Err(error) = persist(items.clone()) {
        queue.pop_back();
        return Err(error);
    }
    let _ = endpoint.ui.send(UiEvent::QueuedPrompts { items });
    permit.send(UiCommand::DrainQueue);
    Ok(())
}

fn remove_queued(
    endpoint: &Endpoint,
    index: usize,
    expected: &[String],
    persist: impl FnOnce(Vec<String>) -> Result<(), String>,
) -> Result<(), String> {
    update_queued(endpoint, expected, persist, |queue| {
        queue
            .remove(index)
            .ok_or_else(|| "排队消息不存在".to_string())?;
        Ok(())
    })
}

fn update_queued(
    endpoint: &Endpoint,
    expected: &[String],
    persist: impl FnOnce(Vec<String>) -> Result<(), String>,
    change: impl FnOnce(&mut VecDeque<String>) -> Result<(), String>,
) -> Result<(), String> {
    let mut queue = endpoint.queue.lock().map_err(|error| error.to_string())?;
    if !queue.iter().eq(expected.iter()) {
        let _ = endpoint.ui.send(UiEvent::QueuedPrompts {
            items: queue.iter().cloned().collect(),
        });
        return Err("队列已变化，请检查后重试".into());
    }
    // Prepare a copy while holding the actor's deque lock. Invalid operations
    // or failed persistence cannot partially alter the live queue.
    let mut updated = queue.clone();
    change(&mut updated)?;
    let items: Vec<_> = updated.iter().cloned().collect();
    persist(items.clone())?;
    *queue = updated;
    let _ = endpoint.ui.send(UiEvent::QueuedPrompts { items });
    Ok(())
}

fn edit_queued(
    endpoint: &Endpoint,
    index: usize,
    expected: &[String],
    text: String,
    persist: impl FnOnce(Vec<String>) -> Result<(), String>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("消息不能为空".into());
    }
    update_queued(endpoint, expected, persist, |queue| {
        let item = queue
            .get_mut(index)
            .ok_or_else(|| "排队消息不存在".to_string())?;
        *item = text;
        Ok(())
    })
}

fn move_queued(
    endpoint: &Endpoint,
    from: usize,
    to: usize,
    expected: &[String],
    persist: impl FnOnce(Vec<String>) -> Result<(), String>,
) -> Result<(), String> {
    update_queued(endpoint, expected, persist, |queue| {
        if from >= queue.len() || to >= queue.len() {
            return Err("无效的排队消息位置".into());
        }
        let item = queue.remove(from).expect("validated queue index");
        queue.insert(to, item);
        Ok(())
    })
}

fn resume_queued(endpoint: &Endpoint) -> Result<(), String> {
    let permit = endpoint
        .cmd
        .try_reserve()
        .map_err(|error| error.to_string())?;
    let queue = endpoint.queue.lock().map_err(|error| error.to_string())?;
    if queue.is_empty() {
        return Err("没有待发送消息".into());
    }
    permit.send(UiCommand::DrainQueue);
    Ok(())
}

fn execute(
    command: Command,
    manager: &mut SessionManager,
    paths: &agent_paths::AppPaths,
    routing: &Arc<Mutex<Routing>>,
) -> Result<(), String> {
    let refresh_settings = matches!(
        &command,
        Command::SetThemeMode(_)
            | Command::SetDesktopNotifications(_)
            | Command::SetMemoryEnabled(_)
            | Command::SetMemoryDistill(_)
            | Command::ReloadConfig
    );
    match command {
        Command::Activity(_) => return Err("活动请求必须由专用 owner 路由执行".into()),
        Command::Settings(command) => {
            super::settings::execute(command, manager, paths)?;
            let settings = super::settings::snapshot(manager, paths);
            routing.lock().map_err(|e| e.to_string())?.settings = Some(settings);
        }
        Command::Refresh => {}
        Command::NewSession => {
            if !manager.workspace_active {
                return Err("open a workspace first".into());
            }
            manager.new_session();
        }
        Command::OpenSession(key) => {
            prepare_history(std::path::Path::new(&key.root), Some(key.id), routing);
            select_workspace(manager, &key)?;
            manager.open_session(key.id);
        }
        Command::DeleteSession(key) => {
            require_known_session(manager, &key)?;
            if !manager.workspace_active || manager.workspace_root.to_string_lossy() != key.root {
                return Err("switch to the session's workspace before deleting it".into());
            }
            manager.delete_session(key.id);
            if manager.store_meta(key.id).is_some() {
                return Err("session could not be deleted".into());
            }
            let mut state = routing.lock().map_err(|e| e.to_string())?;
            state.endpoints.remove(&key);
            state.live.remove(&key);
            state.history_loaded.remove(&key);
            state.baselines.remove(&key);
            state.approvals.retain(|(owner, _, _), _| owner != &key);
        }
        Command::SetSessionPinned {
            key,
            pinned,
            global,
        } => {
            require_known_session(manager, &key)?;
            let updated = if global {
                manager.pin_session_global(Some(&key.root), key.id, pinned)
            } else {
                manager.pin_session(Some(&key.root), key.id, pinned)
            }
            .map_err(|error| error.to_string())?;
            if !updated {
                return Err("session no longer exists".into());
            }
        }
        Command::RenameSession { key, title } => {
            require_known_session(manager, &key)?;
            let updated = manager
                .rename_session(Some(&key.root), key.id, &title)
                .map_err(|error| error.to_string())?;
            if !updated {
                return Err("session no longer exists".into());
            }
        }
        Command::SwitchWorkspace(path) => {
            if !path.is_dir() && !husk_host::office::is_office_root(&path) {
                return Err("workspace must be an existing directory".into());
            }
            prepare_history(&path, None, routing);
            manager.switch_workspace(path).map_err(|e| e.to_string())?;
        }
        Command::Send { key, text } => {
            if text.trim().is_empty() {
                return Err("message must not be empty".into());
            }
            let endpoint = endpoint(routing, &key)?;
            enqueue(&endpoint, text, |items| persist_queue(&key, items))?;
        }
        Command::RemoveQueued {
            key,
            index,
            expected,
        } => {
            let endpoint = endpoint(routing, &key)?;
            remove_queued(&endpoint, index, &expected, |items| {
                persist_queue(&key, items)
            })?;
        }
        Command::EditQueued {
            key,
            index,
            expected,
            text,
        } => {
            let endpoint = endpoint(routing, &key)?;
            edit_queued(&endpoint, index, &expected, text, |items| {
                persist_queue(&key, items)
            })?;
        }
        Command::MoveQueued {
            key,
            from,
            to,
            expected,
        } => {
            let endpoint = endpoint(routing, &key)?;
            move_queued(&endpoint, from, to, &expected, |items| {
                persist_queue(&key, items)
            })?;
        }
        Command::ResumeQueued { key } => {
            let endpoint = endpoint(routing, &key)?;
            resume_queued(&endpoint)?;
        }
        Command::SetModel {
            key,
            provider,
            model,
        } => {
            require_active_session(manager, &key)?;
            let config =
                agent_llm::AppConfig::load(&paths.config_dir, None).map_err(|e| e.to_string())?;
            let provider_config = config
                .providers
                .get(&provider)
                .ok_or_else(|| "unknown provider".to_string())?;
            if model.trim().is_empty()
                || (!provider_config.models.is_empty()
                    && provider_config.find_model(&model).is_none()
                    && provider_config.default_model.as_deref() != Some(model.as_str()))
            {
                return Err("unknown model for this provider".into());
            }
            active_command(
                manager,
                UiCommand::SetModel {
                    provider: provider.clone(),
                    model: model.clone(),
                },
            )?;
            if let Some(handle) = manager.active() {
                *handle.model.write().map_err(|e| e.to_string())? =
                    (provider.clone(), model.clone());
            }
            manager.set_model(provider, model);
        }
        Command::SetFastMode { key, enabled } => {
            manager.set_fast_mode(Some(key.id), Some(&key.root), enabled)?;
        }
        Command::SetThinkingLevel { key, level } => {
            require_active_session(manager, &key)?;
            active_command(
                manager,
                UiCommand::SetThinkingLevel {
                    level: level.clone(),
                },
            )?;
            if let Some(handle) = manager.active() {
                *handle.thinking_level.write().map_err(|e| e.to_string())? = if level.is_empty() {
                    None
                } else {
                    Some(level.clone())
                };
            }
            manager.set_thinking_level(level);
        }
        Command::SetPermissionMode { key, mode } => {
            require_active_session(manager, &key)?;
            if !matches!(
                mode.as_str(),
                "default" | "acceptEdits" | "auto" | "dontAsk" | "bypassPermissions"
            ) {
                return Err("unknown permission mode".into());
            }
            active_command(manager, UiCommand::SetPermissionMode { mode: mode.clone() })?;
            if let Some(handle) = manager.active() {
                handle
                    .permissions
                    .write()
                    .map_err(|e| e.to_string())?
                    .set_mode(PermissionMode::from_str(&mode));
            }
            manager.set_permission_mode(mode);
        }
        Command::SetAgentMode { key, mode } => {
            require_active_session(manager, &key)?;
            if !matches!(mode.as_str(), "build" | "plan" | "goal" | "office") {
                return Err("unknown agent mode".into());
            }
            active_command(manager, UiCommand::SetAgentMode { mode: mode.clone() })?;
            if let Some(handle) = manager.active() {
                *handle.agent_mode.write().map_err(|e| e.to_string())? = mode.clone();
            }
            manager.set_agent_mode(mode);
        }
        Command::SetThemeMode(mode) => {
            if !matches!(mode.as_str(), "system" | "light" | "dark") {
                return Err("unknown theme mode".into());
            }
            let mut appearance = husk_host::prefs::load_appearance_settings();
            appearance.theme_mode = mode;
            husk_host::prefs::save_appearance_settings(&appearance)
                .map_err(|error| error.to_string())?;
        }
        Command::SetDesktopNotifications(enabled) => {
            husk_host::prefs::set_desktop_notifications_enabled(enabled)
                .map_err(|error| error.to_string())?;
        }
        Command::SetMemoryEnabled(enabled) => {
            manager
                .set_memory_prefs_checked(Some(enabled), None)
                .map_err(|error| error.to_string())?;
        }
        Command::SetMemoryDistill(distill) => {
            manager
                .set_memory_prefs_checked(None, Some(distill))
                .map_err(|error| error.to_string())?;
        }
        Command::ReloadConfig => {
            let config =
                agent_llm::AppConfig::load(&paths.config_dir, None).map_err(|e| e.to_string())?;
            let (notified, live) = manager.apply_loaded_config(config);
            if notified != live {
                return Err(format!(
                    "configuration loaded, but only {notified}/{live} live sessions accepted reload; retry"
                ));
            }
        }
        Command::Cancel { .. }
        | Command::Decide { .. }
        | Command::RevokeSessionTool { .. }
        | Command::AnswerQuestion { .. } => {
            return Err("control command reached the owner queue".into());
        }
    }
    if refresh_settings {
        let settings = super::settings::snapshot(manager, paths);
        routing.lock().map_err(|e| e.to_string())?.settings = Some(settings);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_kernel::tools::registry::AskOutcome;

    fn queue_endpoint() -> (Endpoint, tokio::sync::mpsc::Receiver<UiCommand>) {
        let (ui, _events) = UiSink::channel();
        let (cmd, receiver) = tokio::sync::mpsc::channel(1);
        (
            Endpoint {
                cmd,
                decision: Arc::default(),
                pending: Arc::default(),
                cancel: Arc::default(),
                shutdown: Arc::default(),
                permissions: Arc::new(RwLock::new(PermissionGate::from_mode_str("default"))),
                ask: Arc::new(AskChannel::new(Some(ui.clone()))),
                ui,
                queue: Arc::default(),
            },
            receiver,
        )
    }

    #[test]
    fn queue_submission_rolls_back_when_persistence_or_drain_fails() {
        let (endpoint, mut commands) = queue_endpoint();
        assert!(enqueue(&endpoint, "first".into(), |_| Err("disk error".into())).is_err());
        assert!(endpoint.queue.lock().unwrap().is_empty());
        assert!(commands.try_recv().is_err());

        enqueue(&endpoint, "first".into(), |items| {
            assert_eq!(items, ["first"]);
            assert!(endpoint.queue.try_lock().is_err());
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            UiCommand::DrainQueue
        ));
        endpoint.cmd.try_send(UiCommand::DrainQueue).unwrap();
        assert!(
            enqueue(&endpoint, "second".into(), |_| panic!(
                "queue must not be persisted"
            ))
            .is_err()
        );
        assert_eq!(
            endpoint.queue.lock().unwrap().iter().collect::<Vec<_>>(),
            ["first"]
        );
    }

    #[test]
    fn queue_removal_rejects_stale_state_and_restores_on_write_failure() {
        let (endpoint, _) = queue_endpoint();
        *endpoint.queue.lock().unwrap() = ["first".to_owned(), "second".to_owned()].into();
        let stale = vec!["first".to_owned()];
        assert!(remove_queued(&endpoint, 0, &stale, |_| panic!("stale write")).is_err());
        let expected = vec!["first".to_owned(), "second".to_owned()];
        assert!(remove_queued(&endpoint, 0, &expected, |_| Err("disk error".into())).is_err());
        assert_eq!(
            endpoint.queue.lock().unwrap().iter().collect::<Vec<_>>(),
            ["first", "second"]
        );
        remove_queued(&endpoint, 0, &expected, |items| {
            assert_eq!(items, ["second"]);
            assert!(endpoint.queue.try_lock().is_err());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            endpoint.queue.lock().unwrap().iter().collect::<Vec<_>>(),
            ["second"]
        );
    }

    #[test]
    fn queue_edits_preserve_other_items_and_reject_invalid_or_stale_writes() {
        let (endpoint, _) = queue_endpoint();
        let expected = vec!["first".to_owned(), "second".to_owned()];
        *endpoint.queue.lock().unwrap() = expected.clone().into();
        edit_queued(&endpoint, 1, &expected, "edited".into(), |items| {
            assert_eq!(items, ["first", "edited"]);
            assert!(endpoint.queue.try_lock().is_err());
            Ok(())
        })
        .unwrap();
        assert!(
            edit_queued(&endpoint, 0, &expected, "stale".into(), |_| {
                panic!("stale queue must not be persisted")
            })
            .is_err()
        );
        let current = vec!["first".to_owned(), "edited".to_owned()];
        assert!(
            edit_queued(&endpoint, 1, &current, "unsaved".into(), |_| {
                Err("disk error".into())
            })
            .is_err()
        );
        for (index, text) in [(2, "invalid index"), (0, " \n ")] {
            assert!(
                edit_queued(&endpoint, index, &current, text.into(), |_| {
                    panic!("invalid edit must not be persisted")
                })
                .is_err()
            );
        }
        assert_eq!(*endpoint.queue.lock().unwrap(), VecDeque::from(current));
    }

    #[test]
    fn queue_moves_keep_order_and_duplicates_without_losing_payloads() {
        let (endpoint, _) = queue_endpoint();
        let expected = vec![
            "same".to_owned(),
            "attachment payload".to_owned(),
            "same".to_owned(),
        ];
        *endpoint.queue.lock().unwrap() = expected.clone().into();
        move_queued(&endpoint, 2, 0, &expected, |items| {
            assert_eq!(items, ["same", "same", "attachment payload"]);
            Ok(())
        })
        .unwrap();
        let current = vec![
            "same".to_owned(),
            "same".to_owned(),
            "attachment payload".to_owned(),
        ];
        assert!(move_queued(&endpoint, 0, 2, &expected, |_| panic!("stale queue")).is_err());
        assert!(move_queued(&endpoint, 0, 3, &current, |_| panic!("invalid index")).is_err());
        assert!(move_queued(&endpoint, 0, 2, &current, |_| Err("disk error".into())).is_err());
        assert_eq!(
            *endpoint.queue.lock().unwrap(),
            VecDeque::from(current.clone())
        );
        move_queued(&endpoint, 0, 2, &current, |items| {
            assert_eq!(items, ["same", "attachment payload", "same"]);
            Ok(())
        })
        .unwrap();
        assert_eq!(*endpoint.queue.lock().unwrap(), VecDeque::from(expected));
    }

    #[test]
    fn resume_requires_a_nonempty_queue_and_does_not_remove_items() {
        let (endpoint, mut commands) = queue_endpoint();
        assert!(resume_queued(&endpoint).is_err());
        assert!(commands.try_recv().is_err());
        endpoint.queue.lock().unwrap().push_back("queued".into());
        resume_queued(&endpoint).unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            UiCommand::DrainQueue
        ));
        assert_eq!(
            endpoint.queue.lock().unwrap().iter().collect::<Vec<_>>(),
            ["queued"]
        );
    }

    #[test]
    fn answer_delivery_acknowledges_only_the_accepted_request() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (ui, mut ui_events) = UiSink::channel();
            let ask = Arc::new(AskChannel::new(Some(ui.clone())));
            let asking = {
                let ask = ask.clone();
                tokio::spawn(async move { ask.ask("Question".into(), Vec::new(), None).await })
            };
            let question_id = match tokio::time::timeout(Duration::from_secs(5), ui_events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                UiEvent::QuestionAsked { request_id, .. } => request_id,
                other => panic!("unexpected event: {other:?}"),
            };
            let key = SessionKey {
                root: "/test".into(),
                id: 1,
            };
            let (cmd, _cmd_rx) = tokio::sync::mpsc::channel(1);
            let endpoint = Endpoint {
                cmd,
                decision: Arc::default(),
                pending: Arc::default(),
                cancel: Arc::default(),
                shutdown: Arc::default(),
                permissions: Arc::new(RwLock::new(PermissionGate::from_mode_str("default"))),
                ask,
                ui,
                queue: Arc::default(),
            };
            let mut state = Routing::default();
            state.endpoints.insert(key.clone(), endpoint);
            let routing = Arc::new(Mutex::new(state));
            let (events, received) = async_channel::unbounded();
            let stopped = Arc::new(AtomicBool::new(false));
            control(
                101,
                Command::AnswerQuestion {
                    key: key.clone(),
                    request_id: question_id,
                    answer: "accepted".into(),
                },
                &routing,
                &events,
                &stopped,
            );
            assert!(matches!(
                received.try_recv().unwrap(),
                BridgeEvent::AnswerDelivered { request_id: 101 }
            ));
            assert!(matches!(
                ui_events.recv().await,
                Some(UiEvent::QuestionAnswered { request_id }) if request_id == question_id
            ));
            assert!(matches!(
                asking.await.unwrap().unwrap(),
                AskOutcome::Answered(answer) if answer == "accepted"
            ));
            control(
                102,
                Command::AnswerQuestion {
                    key,
                    request_id: question_id,
                    answer: "late".into(),
                },
                &routing,
                &events,
                &stopped,
            );
            assert!(matches!(
                received.try_recv().unwrap(),
                BridgeEvent::Error { request_id: 102, message }
                    if message.contains("no longer pending")
            ));
        });
    }
}
