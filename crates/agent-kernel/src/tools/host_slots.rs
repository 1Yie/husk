//! Host-owned capability slots on [`ToolCtx`].
//!
//! Every slot here is a capability the *host* decides whether to provide:
//! desktop control, a process sandbox, the question channel, the UI sink, the
//! workspace memory handle, the owning session, the plugin router. The generic
//! contract in `registry` knows none of them — it stores whatever the host
//! installs in `Extensions` and refuses a tool whose capability is missing.
//!
//! Keeping these out of `registry` is what lets the tool contract be extracted
//! without naming product types.

use std::path::PathBuf;
use std::sync::Arc;

use agent_paths::AppPaths;

use super::registry::{Extensions, ToolCtx, ToolError};

/// The structured-question channel — `ask_question` parks a oneshot here,
/// emits `QuestionAsked` through `ui_tx`, and awaits the answer with the
/// same cancel poll as `wait_for_decision`. Headless contexts get
/// `ui_tx: None` → the tool refuses instead of deadlocking.
pub struct AskChannel {
    /// The session's UI event channel — `None` where nobody can answer
    /// (delegated subagents, tests).
    pub ui_tx: Option<crate::channels::UiSink>,
    pending: std::sync::Mutex<Option<(u64, tokio::sync::oneshot::Sender<String>)>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl AskChannel {
    pub fn new(ui_tx: Option<crate::channels::UiSink>) -> Self {
        Self {
            ui_tx,
            pending: std::sync::Mutex::new(None),
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Interactive? `None` ui_tx means headless (subagent, test) — the
    /// tool checks this BEFORE building a request so a child agent can't
    /// park on a oneshot nobody can ever answer.
    pub fn is_available(&self) -> bool {
        self.ui_tx.is_some()
    }

    /// Emit the card, park the answer oneshot, await it (or cancellation).
    pub async fn ask(
        &self,
        question: String,
        options: Vec<agent_ipc::events::AskOption>,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<String, ToolError> {
        let Some(ui_tx) = &self.ui_tx else {
            return Err(ToolError::Failed(
                "ask_question unavailable — no UI channel in this context".into(),
            ));
        };
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.pending.lock().unwrap() = Some((id, tx));
        if ui_tx
            .send(agent_ipc::events::UiEvent::QuestionAsked {
                request_id: id,
                question,
                options,
            })
            .is_err()
        {
            *self.pending.lock().unwrap() = None;
            return Err(ToolError::Failed(
                "ask_question failed — UI event channel is closed".into(),
            ));
        }
        let mut rx = std::pin::pin!(rx);
        loop {
            tokio::select! {
                r = &mut rx => {
                    return r.map_err(|_| ToolError::Failed(
                        "question channel closed before an answer arrived".into(),
                    ));
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if cancel.as_ref().is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
                        *self.pending.lock().unwrap() = None;
                        return Err(ToolError::Failed("cancelled".into()));
                    }
                }
            }
        }
    }

    /// `AnswerQuestion` writes here — resolves the parked oneshot when the
    /// id matches (a stale answer for a cleared question is ignored).
    pub fn answer(&self, request_id: u64, answer: String) -> bool {
        match self.pending.lock().unwrap().take() {
            Some((id, tx)) if id == request_id => tx.send(answer).is_ok(),
            Some(p) => {
                *self.pending.lock().unwrap() = Some(p);
                false
            }
            None => false,
        }
    }

    /// Cancel path — dropping the oneshot wakes `ask()` with an error.
    pub fn clear(&self) {
        *self.pending.lock().unwrap() = None;
    }
}

/// Shared, swappable handle to the workspace memory store. Cloning shares the
/// same slot, so a subagent or a tool sees the owning actor's current switch
/// state rather than a snapshot from spawn time.
#[derive(Clone, Default)]
pub struct MemoryHandle(
    pub(crate) Arc<std::sync::RwLock<Option<Arc<agent_context::memory::MemoryStore>>>>,
);

impl MemoryHandle {
    /// The store as of now — `None` while memory is switched off.
    pub fn get(&self) -> Option<Arc<agent_context::memory::MemoryStore>> {
        self.0.read().ok().and_then(|g| g.clone())
    }

    /// Replace the store (`None` switches memory off).
    pub fn set(&self, store: Option<Arc<agent_context::memory::MemoryStore>>) {
        if let Ok(mut g) = self.0.write() {
            *g = store;
        }
    }
}

/// Newtype key for the desktop backend. The extension table is keyed by the
/// concrete type, and `dyn DesktopBackend` is not one type per implementation,
/// so the trait object is wrapped to give the slot a single identity.
pub struct DesktopSlot(pub Arc<dyn agent_computer::DesktopBackend>);

/// Newtype key for the process sandbox, same reason as [`DesktopSlot`].
///
/// Installed by the host. Absent means the host runs no local sandbox at all
/// (an SSH worker); process tools then refuse rather than run unisolated by
/// accident. An installed `none` backend is a different, deliberate state:
/// the user turned the sandbox off, and `bash` still runs, loudly.
pub struct SandboxSlot(pub Arc<dyn agent_sandbox::SandboxBackend>);

/// The host's sandbox overrides (memory, processes, network policy). Optional:
/// a context without one runs the audit's own defaults, which is what a host
/// that never configured limits should get.
pub struct SandboxLimitsSlot(pub crate::sandbox_prefs::SandboxLimits);

/// The interactive question channel. Installed by the host that owns a user
/// to answer; absent means headless (a subagent, a test), and `ask_question`
/// refuses instead of parking on an answer nobody can give.
pub struct AskSlot(pub Arc<AskChannel>);

/// The session UI event sink. Tools that fan out (batch items, subagent
/// progress) emit their internal calls here so they stay visible and
/// auditable instead of running silently; absent in headless contexts.
pub struct UiSlot(pub crate::channels::UiSink);

/// Subagent spawner — `delegate` runs a fresh-context child engine. Absent
/// where delegation is unavailable (tests, child contexts).
pub struct SubagentSlot(pub crate::tools::delegate::SubagentSpawner);

/// The workspace memory handle. Installed by the owning session and shared
/// with its actor, which swaps the store when the memory switch flips. Absent
/// where no session owns a store; `remember` then refuses.
pub struct MemorySlot(pub MemoryHandle);

/// The owning session's id and per-workspace store. Tools that keep
/// per-session scratch state (`todo`) write next to the session's history
/// file; `delegate` uses the id to route steering to the right owner.
pub struct SessionSlot(pub (i64, Arc<crate::session_store::SessionStore>));

/// Plugin router handle. `batch_execute` consults it to tell a plugin tool from
/// an unknown name. Absent where no plugin router is installed.
pub struct PluginSlot(pub crate::engine::PluginHandle);

/// The host's app directories. Tools that keep workspace-local state
/// (attachments, subagent transcripts, repo-local agent manifests) derive
/// their directory from the installed workspace state dir name instead of
/// hardcoding the product's.
pub struct AppPathsSlot(pub AppPaths);

impl ToolCtx {
    /// The host's app directories, if installed.
    pub fn paths(&self) -> Option<AppPaths> {
        self.ext.get::<AppPathsSlot>().map(|s| s.0.clone())
    }

    /// Install the host's app directories.
    pub fn install_paths(&mut self, paths: AppPaths) {
        self.ext.insert(Arc::new(AppPathsSlot(paths)));
    }

    /// The host's app paths, or a product-free default for a bare context
    /// (tests, headless spawns): config and data under `<ws>/.agent`, and no
    /// user-level dirs are reachable through it. Tools that must have *some*
    /// paths use this instead of unwrapping the slot.
    pub fn app_paths(&self) -> AppPaths {
        self.paths().unwrap_or_else(|| {
            AppPaths::new(
                self.workspace_root.join(".agent/config"),
                self.workspace_root.join(".agent/data"),
                ".agent",
            )
        })
    }

    /// The workspace-local state dir (`<workspace>/.husk` for the husk host).
    ///
    /// Falls back to a product-free `.agent` when no paths are installed, so a
    /// bare test context still gets a defined, private-looking directory.
    pub fn workspace_state_dir(&self) -> PathBuf {
        self.paths()
            .map(|p| p.workspace_dir(&self.workspace_root))
            .unwrap_or_else(|| self.workspace_root.join(".agent"))
    }

    /// The question channel, or a refusal when the context has no one to ask.
    pub fn ask(&self) -> Result<Arc<AskChannel>, ToolError> {
        self.ext
            .get::<AskSlot>()
            .map(|s| s.0.clone())
            .ok_or_else(|| {
                ToolError::Failed("ask_question requires an interactive user session".into())
            })
    }

    /// Install the question channel. Hosts keep their own handle to the same
    /// `Arc`, so answers routed through the host reach the parked tool call.
    pub fn install_ask(&mut self, ask: Arc<AskChannel>) {
        self.ext.insert(Arc::new(AskSlot(ask)));
    }

    /// The session UI event sink, if this context has one. Tools that fan out
    /// emit their internal call events here; `None` means headless, where they
    /// skip emitting rather than inventing a sink.
    pub fn ui(&self) -> Option<crate::channels::UiSink> {
        self.ext.get::<UiSlot>().map(|s| s.0.clone())
    }

    /// Install the session UI event sink.
    pub fn install_ui(&mut self, ui: crate::channels::UiSink) {
        self.ext.insert(Arc::new(UiSlot(ui)));
    }

    /// The workspace memory handle, if a session owns one. The handle itself
    /// may hold `None` while memory is switched off; that is a refusal at the
    /// tool, not a missing slot.
    pub fn memory(&self) -> Option<MemoryHandle> {
        self.ext.get::<MemorySlot>().map(|s| s.0.clone())
    }

    /// Install the workspace memory handle.
    pub fn install_memory(&mut self, memory: MemoryHandle) {
        self.ext.insert(Arc::new(MemorySlot(memory)));
    }

    /// Share the parent's memory handle with a child, so a subagent's
    /// `remember` lands in the same workspace store.
    pub fn inherit_memory(mut self, parent: &ToolCtx) -> Self {
        if let Some(slot) = parent.ext.get::<MemorySlot>() {
            self.ext.insert(slot);
        }
        self
    }

    /// The subagent spawner, if this context can delegate.
    pub fn subagent(&self) -> Option<Arc<SubagentSlot>> {
        self.ext.get::<SubagentSlot>()
    }

    /// Install the spawner that backs `delegate`.
    pub fn install_subagent(&mut self, spawner: crate::tools::delegate::SubagentSpawner) {
        self.ext.insert(Arc::new(SubagentSlot(spawner)));
    }

    /// The host's sandbox overrides, or the defaults when none were installed.
    pub fn sandbox_limits(&self) -> crate::sandbox_prefs::SandboxLimits {
        self.ext
            .get::<SandboxLimitsSlot>()
            .map(|s| s.0.clone())
            .unwrap_or_default()
    }

    /// The sandbox a process tool dispatches through. The host decides which
    /// backend is installed (the detected one, a `none` backend when the user
    /// turned isolation off, or nothing at all); this only reads what it
    /// installed. Refuses when nothing is installed, so `bash` can never run
    /// unisolated by omission.
    pub fn sandbox(&self) -> Result<Arc<dyn agent_sandbox::SandboxBackend>, ToolError> {
        self.ext
            .get::<SandboxSlot>()
            .map(|s| s.0.clone())
            .ok_or_else(|| {
                ToolError::Failed("shell execution is not available in this context".into())
            })
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        Self {
            workspace_root: Arc::from(root.as_path()),
            cancel: None,
            depth: 0,
            active_registry: std::sync::RwLock::new(None),
            ext: Extensions::default(),
        }
        .with_controllers()
    }

    /// Install a fresh goal and plan controller. Each root context owns its
    /// own pair; a subagent calls this on its own ctx so it never shares the
    /// parent's completion signal.
    pub fn with_controllers(mut self) -> Self {
        self.ext.insert(Arc::new(crate::tools::goal::GoalController::new()));
        self.ext.insert(Arc::new(crate::tools::plan::PlanController::new()));
        self
    }

    /// Explicit backend (tests / custom wiring): a context with `sandbox`
    /// installed as its process sandbox.
    pub fn with_sandbox(
        root: impl Into<PathBuf>,
        sandbox: Arc<dyn agent_sandbox::SandboxBackend>,
    ) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        let mut ctx = Self {
            workspace_root: Arc::from(root.as_path()),
            cancel: None,
            depth: 0,
            active_registry: std::sync::RwLock::new(None),
            ext: Extensions::default(),
        }
        .with_controllers();
        ctx.ext.insert(Arc::new(SandboxSlot(sandbox)));
        ctx
    }

    /// Install the desktop-control backend the computer-use tools drive.
    ///
    /// Optional by design: a host with no display (a remote SSH worker, a
    /// headless CI run) simply doesn't install one, and `computer` /
    /// `screenshot` refuse with a reason instead of the kernel probing hardware.
    ///
    /// Shared across clones: `with_cancel` hands out the same `Arc`, whose
    /// `last` capture drives coordinate mapping for every `computer` call. A
    /// screenshot taken in one tool call steers the click in the next, so a
    /// `batch_execute` that fans out `screenshot`+`computer` items must keep
    /// them sequenced, or the click can map onto the previous frame.
    pub fn with_desktop(mut self, desktop: Arc<dyn agent_computer::DesktopBackend>) -> Self {
        self.ext.insert(Arc::new(DesktopSlot(desktop)));
        self
    }

    /// Hand the parent's desktop backend to a child context. Only this slot is
    /// copied: the goal and plan controllers must stay per-context, so the
    /// child never inherits the parent's `ext` wholesale.
    pub fn inherit_desktop(mut self, parent: &ToolCtx) -> Self {
        if let Some(slot) = parent.ext.get::<DesktopSlot>() {
            self.ext.insert(slot);
        }
        self
    }

    /// Install the process sandbox on an existing context. `with_sandbox` is
    /// the builder form; hosts that assemble a context in steps use this.
    pub fn install_sandbox(&mut self, sandbox: Arc<dyn agent_sandbox::SandboxBackend>) {
        self.ext.insert(Arc::new(SandboxSlot(sandbox)));
    }

    /// Install the host's sandbox overrides. Optional: without this the audit's
    /// own defaults apply.
    pub fn with_sandbox_limits(mut self, limits: crate::sandbox_prefs::SandboxLimits) -> Self {
        self.ext.insert(Arc::new(SandboxLimitsSlot(limits)));
        self
    }

    /// Hand the parent's process sandbox and its limits to a child context.
    /// The limits travel with the sandbox on purpose: a child's `bash` must
    /// obey the host's forced network policy and caps, or a subagent would be
    /// the one place the host's settings stop applying.
    pub fn inherit_sandbox(mut self, parent: &ToolCtx) -> Self {
        if let Some(slot) = parent.ext.get::<SandboxSlot>() {
            self.ext.insert(slot);
        }
        if let Some(slot) = parent.ext.get::<SandboxLimitsSlot>() {
            self.ext.insert(slot);
        }
        self
    }

    /// The installed desktop backend. Refuses with the same wording as the
    /// other missing services when the host did not install one.
    pub fn desktop(&self) -> Result<Arc<dyn agent_computer::DesktopBackend>, ToolError> {
        self.ext
            .get::<DesktopSlot>()
            .map(|s| s.0.clone())
            .ok_or_else(|| ToolError::Failed("desktop control is not available in this context".into()))
    }

    /// Attach the owning session — gives tools access to per-session
    /// scratch files in the app state dir.
    pub fn with_session(
        mut self,
        id: i64,
        store: Option<Arc<crate::session_store::SessionStore>>,
    ) -> Self {
        if let Some(store) = store {
            self.ext.insert(Arc::new(SessionSlot((id, store))));
        }
        self
    }

    /// The owning session's id and its per-workspace store, if attached.
    pub fn session(&self) -> Option<(i64, Arc<crate::session_store::SessionStore>)> {
        self.ext.get::<SessionSlot>().map(|s| s.0.clone())
    }

    /// The plugin router handle, if one is installed.
    pub fn plugins(&self) -> Option<crate::engine::PluginHandle> {
        self.ext.get::<PluginSlot>().map(|s| s.0.clone())
    }

    /// The engine's `set_plugins` replaces its `Arc<ToolCtx>` wholesale —
    /// no interior mutability on the ctx itself.
    pub fn with_plugins(&self, plugins: crate::engine::PluginHandle) -> ToolCtx {
        let cancel = self
            .cancel
            .clone()
            .unwrap_or_else(|| Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let mut next = self.with_cancel(cancel);
        next.ext.insert(Arc::new(PluginSlot(plugins)));
        next
    }
}
