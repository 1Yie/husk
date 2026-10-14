//! `ToolSpec` + dispatch — one registry for built-ins and plugin tools.
//!
//! A spec carries the name, the JSON Schema sent to the model, the `readonly`
//! flag (skips confirmation in `default` mode) and the exec fn. The registry
//! produces the request's `tools` array and dispatches by name; plugin tools
//! merge in under `plugin_id:name` (Stage 9).

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;

/// Arguments as delivered by the model — a JSON object.
pub type Args = serde_json::Value;

/// Shared context handed to every tool invocation.

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
pub struct MemoryHandle(Arc<std::sync::RwLock<Option<Arc<agent_context::memory::MemoryStore>>>>);

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

/// Typed extension slots on a [`ToolCtx`]. A tool that needs a host-owned
/// service reads it by type, so `ToolCtx` itself carries no product fields and
/// the tool-contract crate does not have to name the types that provide them.
///
/// Keyed by `TypeId`: one value per type. Cloning shares the same `Arc`s, so a
/// `with_cancel` copy of the ctx sees the same controller as its parent.
#[derive(Clone, Default)]
pub struct Extensions {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Extensions {
    pub fn insert<T: Any + Send + Sync>(&mut self, value: Arc<T>) {
        self.map.insert(TypeId::of::<T>(), value);
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|v| Arc::clone(v).downcast::<T>().ok())
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

pub struct ToolCtx {
    /// Canonical workspace root — all path args resolve and are checked
    /// against this (symlink-escape guard: canonicalize before compare).
    pub workspace_root: Arc<Path>,
    /// Parent turn's cooperative cancel flag — a delegated subagent polls
    /// it so a user cancel propagates into the delegation instead of
    /// orphaning a running child. `None` in unattached contexts.
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Session UI event channel — tools that fan out (batch items,
    /// subagent progress) emit ToolCallStarted/Finished here so internal
    /// calls stay visible + auditable instead of running silently.
    /// `None` in headless contexts → those tools skip emitting.
    pub ui_tx: Option<crate::channels::UiSink>,
    /// Live view of the engine's active registry — refreshed on every
    /// `active_registry()` resolution so `batch_execute` dispatches through
    /// the same mode-scoped registry as single calls (plan mode can't be
    /// bypassed by batching a captured full registry).
    pub active_registry: std::sync::RwLock<Option<Arc<ToolRegistry>>>,
    /// Delegation depth — `delegate` bumps it in the child's ctx and the
    /// tool refuses past MAX_SUBAGENT_DEPTH, a second guard behind the
    /// registry-level exclusion of `delegate`.
    pub depth: u8,
    /// Typed host services — the goal and plan controllers live here, not as
    /// fields, so this crate does not name the modules that own them. See
    /// [`Extensions`]; the `GoalController`/`PlanController` slots are
    /// installed by [`ToolCtx::with_controllers`].
    pub ext: Extensions,
}

impl ToolCtx {
    /// The host's sandbox overrides, or the defaults when none were installed.
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
            ui_tx: None,
            ext: Extensions::default(),
        }
        .with_controllers()
    }

    /// A required host service. A context built without it refuses the tool
    /// with a reason instead of panicking mid-turn.
    pub fn require<T: Any + Send + Sync>(&self, what: &str) -> Result<Arc<T>, ToolError> {
        self.ext
            .get::<T>()
            .ok_or_else(|| ToolError::Failed(format!("{what} is not available in this context")))
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
            ui_tx: None,
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

    /// Resolve a model-supplied path against the workspace root and prove it
    /// stays inside. Returns the canonical path or a sandbox-escape error.
    /// Same context with a different cancel flag — `batch_execute` scopes
    /// items to a batch deadline flag so a timeout can abort in-flight
    /// blocking work without touching the turn's cancel.
    pub fn with_cancel(&self, cancel: Arc<std::sync::atomic::AtomicBool>) -> ToolCtx {
        ToolCtx {
            workspace_root: self.workspace_root.clone(),
            cancel: Some(cancel),
            ui_tx: self.ui_tx.clone(),
            active_registry: std::sync::RwLock::new(self.active_registry.read().unwrap().clone()),
            depth: self.depth,
            ext: self.ext.clone(),
        }
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

    pub fn resolve(&self, path: &str) -> Result<PathBuf, ToolError> {
        let p = Path::new(path);
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace_root.join(p)
        };
        // Canonicalize requires the path to exist; for writes to new files,
        // canonicalize the deepest existing ancestor and re-attach the
        // missing tail. Checking only the immediate parent made any
        // `<new-dir>/file` path report PathEscape.
        let canon = match joined.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                let mut tail: Vec<std::ffi::OsString> = Vec::new();
                let mut cursor: &Path = joined.as_path();
                let base = loop {
                    if let Ok(c) = cursor.canonicalize() {
                        break c;
                    }
                    // `canonicalize` also fails for a *dangling* symlink
                    // (its target does not exist). Treating that as a
                    // plain missing path would return the in-workspace
                    // link path, and the later write follows the link to
                    // wherever it points — possibly outside the workspace.
                    if cursor
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                    {
                        return Err(ToolError::PathEscape(path.to_string()));
                    }
                    let Some(name) = cursor.file_name() else {
                        return Err(ToolError::PathEscape(path.to_string()));
                    };
                    tail.push(name.to_os_string());
                    let Some(parent) = cursor.parent() else {
                        return Err(ToolError::PathEscape(path.to_string()));
                    };
                    cursor = parent;
                };
                let mut canon = base;
                for seg in tail.iter().rev() {
                    canon.push(seg);
                }
                canon
            }
        };
        if !canon.starts_with(&*self.workspace_root) {
            return Err(ToolError::PathEscape(path.to_string()));
        }
        Ok(canon)
    }
}

/// A deferred write a tool wants the engine to commit — returned instead
/// of writing inside the tool so the engine can apply the change *after*
/// the permission decision and record it in the HunkTracker in one step
/// (P1-c). The tool produces `content`; the engine owns the side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOp {
    /// Create or overwrite `path` with `content`.
    Write,
    /// Remove `path` (`*** Delete File`).
    Delete,
}

#[derive(Debug, Clone)]
pub struct PendingWrite {
    /// Canonical workspace path to write.
    pub path: std::path::PathBuf,
    /// The full new file content to write (ignored for `Delete`).
    pub content: Vec<u8>,
    /// What the engine should do with `path`.
    pub op: WriteOp,
    /// Create parent directories before writing (`*** Add File` to a
    /// nested path). Ignored for `Delete`.
    pub auto_mkdir: bool,
}

impl PendingWrite {
    /// Convenience constructor for the common overwrite case.
    pub fn write(path: std::path::PathBuf, content: Vec<u8>) -> Self {
        Self {
            path,
            content,
            op: WriteOp::Write,
            auto_mkdir: false,
        }
    }
}

/// Tool execution result — `content` is model-facing text (already folded
/// under the truncation budget), `ui` is an optional typed card for the
/// DiffViewer/status strip.
#[derive(Debug)]
pub struct ToolResult {
    pub content: String,
    /// Typed UI card hint (e.g. `"diff"`, `"table"`, `"markdown"`), rendered
    /// as a rich card when present — plugins obey the same contract.
    pub ui_type: Option<&'static str>,
    /// True when the patch matched approximately — approval card must say so.
    pub fuzzy: bool,
    /// Deferred writes the engine commits post-approval — empty for
    /// read-only tools and tools that manage their own side effects. A
    /// multi-file tool (`apply_patch`) stages one entry per file op.
    pub pending_write: Vec<PendingWrite>,
    /// Staged image refs this result carries as model input (a screenshot).
    /// The engine hangs them on the tool message; adapters encode or annotate
    /// them per their wire (only the vision-capable ones get real parts).
    pub images: Vec<agent_llm::types::ImageRef>,
}

impl ToolResult {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ui_type: None,
            fuzzy: false,
            pending_write: Vec::new(),
            images: Vec::new(),
        }
    }
}

/// Errors tools can raise. Message text is model-facing — ambiguity errors
/// must *teach the fix* ("add context lines"), per native-tools.md.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{0}")]
    Args(String),

    #[error("path escapes workspace: {0}")]
    PathEscape(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Failed(String),
}

/// A tool's exec — `Arc<dyn Fn>` so a bridged tool (e.g. a Serena MCP
/// passthrough) can capture its bridge + remote name in a closure, while
/// plain builtins still wrap a free fn.
pub type ExecFn = Arc<
    dyn Fn(Args, Arc<ToolCtx>) -> BoxFuture<'static, Result<ToolResult, ToolError>> + Send + Sync,
>;

/// What a call actually does — the capability seam `batch_execute`,
/// permission policy, and (eventually) audit all key off. `readonly`
/// stays as the auto-approval hint; `class` is the execution taxonomy —
/// a tool can't sneak into an observation batch just by being readonly
/// (ask_question, goal signals, todo all prove the flags differ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    /// Pure reads — filesystem, network fetch, git status. Batch-eligible.
    Observation,
    /// Internal session state (`todo`) or instructions loaded into it
    /// (`skill`) — serial semantics, never batched.
    SessionMutation,
    /// Writes to the workspace (patches, file edits) — permission-gated.
    WorkspaceMutation,
    /// Process execution (bash, test runner).
    Process,
    /// Turn-control signals (goal contract) — the engine consumes them.
    Control,
    /// Blocks on a human decision (ask_question).
    HumanInteraction,
    /// Spawns orchestration (delegate, batch_execute).
    Orchestration,
}

/// Does this tool stage its writes instead of performing them?
///
/// A staged writer's `exec` returns [`ToolResult::pending_write`] and touches
/// no file, so the engine may dispatch it *before* the permission decision to
/// harvest the real diff, fuzzy flag and pre-images for the approval card, then
/// commit the write only after approval (see `run_turn`'s `is_write` path).
/// Dispatching any other tool early would run it for real.
///
/// This is deliberately NOT `class == WorkspaceMutation`. `serena` declares
/// that class but writes directly (its suite edits files in-process), so
/// treating the class as "safe to dry-run" would perform a workspace write
/// before the user approved it. The set is exact: only `fuzzy_patch` and
/// `apply_patch` return `PendingWrite` today.
///
pub fn is_staged_write(name: &str) -> bool {
    matches!(name, "fuzzy_patch" | "apply_patch")
}

/// One tool: name, schema for the LLM, readonly flag, exec fn.
#[derive(Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    /// JSON Schema object describing the tool's parameters.
    pub schema: serde_json::Value,
    /// Read-only tools auto-run in `default` permission mode.
    pub readonly: bool,
    /// Execution class — batch eligibility, policy, and audit key off it.
    pub class: ToolClass,
    /// Crosses the network boundary — batchable only under a tighter
    /// per-batch cap (SSRF/rate-limit amplification guard).
    pub network: bool,
    pub exec: ExecFn,
}

/// Name → spec map. `BTreeMap` keeps the `tools` array deterministic
/// (provider prompt-cache friendly).
#[derive(Default, Clone)]
pub struct ToolRegistry {
    specs: BTreeMap<&'static str, Arc<ToolSpec>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the phase-1 built-in set.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        r.register(crate::tools::fs_read::spec());
        r.register(crate::tools::fs_patch::spec());
        r.register(crate::tools::apply_patch::spec());
        r.register(crate::tools::list_dir::spec());
        r.register(crate::tools::grep::spec());
        r.register(crate::tools::test_runner::spec());
        r.register(crate::tools::bash::spec());
        r.register(crate::tools::services::spec());
        r.register(crate::tools::todo::spec());
        r.register(crate::tools::skill::spec());
        r.register(crate::tools::remember::spec());
        r.register(crate::tools::serena::spec());
        r.register(crate::tools::web_fetch::spec());
        r.register(crate::tools::web_fetch::spec_alias());
        r.register(crate::tools::screenshot::spec());
        r.register(crate::tools::computer::spec());
        r.register(crate::tools::delegate::spec());
        r.register(crate::tools::ask::spec());
        r.register(crate::tools::batch::spec());
        r.register(crate::tools::blueprint::spec());
        r
    }

    /// The registry a delegated child may draw from: builtins minus the tools no
    /// child may hold. `delegate` is the recursion guard, `computer` and
    /// `HumanInteraction` need a human a child cannot reach, and `remember` writes
    /// the persona every later parent turn reads — a child works on untrusted
    /// content, so it must not write that.
    pub fn child_builtins() -> Self {
        Self::with_builtins().filtered(|s| {
            s.name != "delegate"
                && s.name != "remember"
                && s.name != "computer"
                && s.class != ToolClass::HumanInteraction
        })
    }

    pub fn register(&mut self, spec: ToolSpec) {
        self.specs.insert(spec.name, Arc::new(spec));
    }

    /// A view of this registry keeping only tools that satisfy `keep` —
    /// agent modes build their registries from it (`plan` drops every
    /// non-readonly spec, `goal` adds the contract tools on top of full).
    pub fn filtered(&self, keep: impl Fn(&ToolSpec) -> bool) -> Self {
        let mut r = Self::new();
        for (name, spec) in &self.specs {
            if keep(spec) {
                r.specs.insert(name, spec.clone());
            }
        }
        r
    }

    /// Read-only view — every spec whose writes could touch the workspace
    /// drops out. `plan` mode dispatches against this.
    pub fn readonly_only(&self) -> Self {
        self.filtered(|s| s.readonly)
    }

    /// The `tools` array for an LLM request (OpenAI `function` shape).
    pub fn request_schema(&self) -> serde_json::Value {
        let tools: Vec<serde_json::Value> = self
            .specs
            .values()
            .map(|s| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": s.name,
                        "description": s.schema["description"].clone(),
                        "parameters": s.schema["parameters"].clone(),
                    }
                })
            })
            .collect();
        serde_json::Value::Array(tools)
    }

    /// Dispatch by name. Unknown tool → model-facing error string so the
    /// model can self-correct instead of crashing the turn.
    pub async fn dispatch(
        &self,
        name: &str,
        args: Args,
        ctx: Arc<ToolCtx>,
    ) -> Result<ToolResult, ToolError> {
        match self.specs.get(name) {
            Some(spec) => (spec.exec)(args, ctx).await,
            None => Err(ToolError::Failed(format!(
                "unknown tool '{name}' — available: {}",
                self.specs.keys().copied().collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    pub fn is_readonly(&self, name: &str) -> bool {
        self.specs.get(name).map(|s| s.readonly).unwrap_or(false)
    }

    /// Spec lookup — `batch_execute` filters eligible tools through this
    /// before running them itself.
    pub fn spec(&self, name: &str) -> Option<Arc<ToolSpec>> {
        self.specs.get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }
}

/// Helper: pull a required string arg with a teaching error.
pub fn arg_str<'a>(args: &'a Args, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::Args(format!("missing required string arg '{key}'")))
}

/// Helper: pull an optional usize arg.
pub fn arg_usize(args: &Args, key: &str) -> Option<usize> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as usize)
}

/// Derive a JSON Schema `parameters` object from a serde shape via schemars,
/// and attach the tool description separately.
pub fn schema_for<T: schemars::JsonSchema>(description: &str) -> serde_json::Value {
    let params = schemars::schema_for!(T);
    let params = serde_json::to_value(params).unwrap_or_default();
    serde_json::json!({ "description": description, "parameters": params })
}

#[cfg(test)]
mod tests {
    #[derive(Debug, PartialEq)]
    struct Marker(u32);

    #[test]
    fn extension_round_trips_by_type_and_clones_share_it() {
        let mut ext = Extensions::default();
        assert!(ext.get::<Marker>().is_none());
        ext.insert(Arc::new(Marker(7)));
        assert_eq!(ext.get::<Marker>().map(|m| m.0), Some(7));
        // A clone is the same slot: the controller a ctx was built with is
        // the controller its `with_cancel` copy reads.
        let copy = ext.clone();
        assert!(Arc::ptr_eq(&ext.get::<Marker>().unwrap(), &copy.get::<Marker>().unwrap()));
    }

    #[test]
    fn derived_ctx_shares_the_controller_but_a_child_gets_its_own() {
        let parent = ToolCtx::new(std::env::temp_dir());
        let parent_goal = parent.require::<crate::tools::goal::GoalController>("goal").unwrap();
        // `with_cancel` is how batch and the turn scope a ctx: same controller.
        let scoped = parent.with_cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let scoped_goal = scoped.require::<crate::tools::goal::GoalController>("goal").unwrap();
        assert!(Arc::ptr_eq(&parent_goal, &scoped_goal));
        // A subagent builds its own pair: its completion must not reach the parent.
        let child = ToolCtx::with_sandbox(
            std::env::temp_dir(),
            Arc::new(agent_sandbox::NoneBackend),
        )
        .with_controllers();
        let child_goal = child.require::<crate::tools::goal::GoalController>("goal").unwrap();
        assert!(!Arc::ptr_eq(&parent_goal, &child_goal));
    }

    #[test]
    fn ctx_without_controllers_refuses_instead_of_panicking() {
        let bare = ToolCtx {
            ext: Extensions::default(),
            ..ToolCtx::new(std::env::temp_dir())
        };
        let err = bare
            .require::<crate::tools::goal::GoalController>("goal control")
            .err()
            .expect("missing service must be an error");
        assert!(err.to_string().contains("goal control is not available"));
    }

    #[test]
    fn a_context_without_a_desktop_refuses_and_never_probes() {
        // The kernel no longer probes hardware in `new`: a host with no display
        // (an SSH worker, CI) gets a refusal with a reason, not a panic.
        let ctx = ToolCtx::new(std::env::temp_dir());
        let err = ctx.desktop().err().expect("no desktop was installed");
        assert!(err.to_string().contains("desktop control is not available"));
    }

    #[test]
    fn child_inherits_only_the_desktop_not_the_controllers() {
        let parent = ToolCtx::new(std::env::temp_dir()).with_desktop(Arc::new(
            agent_computer::NoneBackend::new("probe"),
        ));
        let child = ToolCtx::new(std::env::temp_dir()).inherit_desktop(&parent);
        // The same backend instance, so a child screenshot steers the parent's
        // coordinate mapping exactly as the parent's own would.
        assert!(Arc::ptr_eq(
            &parent.desktop().unwrap(),
            &child.desktop().unwrap()
        ));
        // And the child still has its own controllers: nothing was copied wholesale.
        let child_goal = child.require::<crate::tools::goal::GoalController>("goal");
        let parent_goal = parent.require::<crate::tools::goal::GoalController>("goal").unwrap();
        assert!(child_goal.is_ok());
        assert!(!Arc::ptr_eq(&parent_goal, &child_goal.unwrap()));
    }

    #[test]
    fn a_context_without_a_sandbox_refuses_shell_execution() {
        // An SSH worker installs no local sandbox. `bash` must refuse, not run
        // unisolated because nobody said otherwise.
        let ctx = ToolCtx::new(std::env::temp_dir());
        let err = ctx.sandbox().err().expect("no sandbox was installed");
        assert!(err.to_string().contains("shell execution is not available"));
    }

    #[test]
    fn an_installed_none_backend_still_runs_and_says_so() {
        // A deliberately disabled sandbox is a different state from an absent
        // one: the backend is present, reports `none`, and commands still run.
        let ctx = ToolCtx::with_sandbox(
            std::env::temp_dir(),
            Arc::new(agent_sandbox::NoneBackend),
        );
        assert_eq!(ctx.sandbox().unwrap().id(), "none");
    }

    /// The staging set is what the engine dry-runs before the approval card, so
    /// a name that is not a real writer, or a real writer left out of the set,
    /// is a safety or UX bug. Pin it against the registry instead of trusting
    /// the list to stay in sync by hand.
    #[test]
    fn staged_write_set_matches_the_registry() {
        let reg = ToolRegistry::with_builtins();
        // Every staged name is a real, registered, non-readonly workspace
        // mutation: the engine will dispatch it before the permission gate.
        for name in ["fuzzy_patch", "apply_patch"] {
            assert!(
                reg.spec(name).is_some(),
                "staged writer {name} is not a registered tool"
            );
            assert!(
                !reg.is_readonly(name),
                "staged writer {name} must not be readonly"
            );
            assert_eq!(
                reg.spec(name).unwrap().class,
                ToolClass::WorkspaceMutation,
                "staged writer {name} must be a workspace mutation"
            );
        }
        // The registry offers no staged writer outside the set. This is the
        // drift guard: a new staging tool must be added to `is_staged_write`.
        let staged: Vec<&str> = ["fuzzy_patch", "apply_patch"].into_iter().collect();
        for spec in reg.specs.values() {
            let in_registry = staged.contains(&spec.name);
            assert_eq!(
                in_registry,
                is_staged_write(spec.name),
                "{} disagrees between the set and the registry",
                spec.name
            );
        }
        // `serena` is the trap: WorkspaceMutation, but it writes directly, so
        // staging it would run the write before approval.
        assert_eq!(
            reg.spec("serena").unwrap().class,
            ToolClass::WorkspaceMutation
        );
        assert!(
            !is_staged_write("serena"),
            "serena writes in-process; it must not be dry-run early"
        );
    }

    #[test]
    fn ask_is_refused_without_a_host_channel_and_shared_after_cancel_scope() {
        let mut ctx = ToolCtx::new(std::env::temp_dir());
        // Headless by default: the tool gets a refusal, not a parked oneshot.
        assert!(matches!(ctx.ask(), Err(ToolError::Failed(_))));

        let channel = Arc::new(AskChannel::new(None));
        ctx.install_ask(channel.clone());
        assert!(Arc::ptr_eq(&ctx.ask().unwrap(), &channel));
        // A batch item's cancel-scoped copy answers on the same channel.
        let scoped = ctx.with_cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        assert!(Arc::ptr_eq(&scoped.ask().unwrap(), &channel));
    }

    #[test]
    fn subagent_is_absent_until_installed_and_shared_after_cancel_scope() {
        let mut ctx = ToolCtx::new(std::env::temp_dir());
        assert!(ctx.subagent().is_none());

        // Never called: the test only checks presence, not delegation.
        struct Idle;
        #[async_trait::async_trait]
        impl agent_llm::LlmProvider for Idle {
            fn id(&self) -> &'static str {
                "idle"
            }
            async fn chat_stream(
                &self,
                _model: &str,
                _messages: &[agent_llm::ChatMessage],
                _tools: Option<serde_json::Value>,
                _temperature: f32,
                _reasoning_effort: Option<&str>,
                _params: &agent_llm::ModelParams,
            ) -> anyhow::Result<agent_llm::BoxStream<agent_llm::StreamChunk>> {
                anyhow::bail!("idle provider")
            }
        }
        let registry = Arc::new(ToolRegistry::new());
        let route = Arc::new(std::sync::RwLock::new(crate::tools::delegate::ChildRoute {
            provider: Arc::new(Idle),
            model: String::new(),
            temperature: 0.0,
            context_window: 0,
            thinking_level: None,
            thinking_level_map: None,
            model_params: agent_llm::ModelParams::default(),
            model_input: Vec::new(),
            models: std::collections::HashMap::new(),
        }));
        ctx.install_subagent(crate::tools::delegate::SubagentSpawner::new(
            route,
            registry.clone(),
            registry,
        ));
        let scoped = ctx.with_cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        assert!(scoped.subagent().is_some());
    }

    #[test]
    fn memory_is_absent_until_installed_and_a_child_sees_the_live_switch() {
        let parent = ToolCtx::new(std::env::temp_dir());
        assert!(parent.memory().is_none());

        let mut parent = parent;
        let handle = MemoryHandle::default();
        parent.install_memory(handle.clone());
        // Installed but switched off: the slot exists, the store is None.
        assert!(parent.memory().unwrap().get().is_none());

        // A child shares the very same handle, so a later switch on the
        // parent's side is visible to the child without re-inheriting.
        let child = ToolCtx::new(std::env::temp_dir()).inherit_memory(&parent);
        assert!(child.memory().is_some());
        assert!(Arc::ptr_eq(
            &child.memory().unwrap().0,
            &parent.memory().unwrap().0,
        ));
    }

    #[test]
    fn session_is_absent_until_attached_and_shared_after_cancel_scope() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::session_store::SessionStore::open_at(dir.path(), &dir.path().join("t.db"))
            .unwrap();
        let ctx = ToolCtx::new(dir.path());
        assert!(ctx.session().is_none());

        let ctx = ctx.with_session(7, Some(Arc::new(store)));
        let (id, _) = ctx.session().unwrap();
        assert_eq!(id, 7);
        let scoped = ctx.with_cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        assert_eq!(scoped.session().map(|(id, _)| id), Some(7));
    }

    #[test]
    fn plugins_are_scoped_to_the_derived_context_not_the_parent() {
        let parent = ToolCtx::new(std::env::temp_dir());
        assert!(parent.plugins().is_none());

        let handle: crate::engine::PluginHandle =
            Arc::new(std::sync::RwLock::new(None));
        let routed = parent.with_plugins(handle.clone());
        // The engine swaps the whole ctx on set_plugins; the parent keeps none.
        assert!(parent.plugins().is_none());
        assert!(Arc::ptr_eq(&routed.plugins().unwrap(), &handle));
    }

    #[test]
    fn child_inherits_the_sandbox_and_its_limits() {
        let parent = ToolCtx::with_sandbox(
            std::env::temp_dir(),
            Arc::new(agent_sandbox::NoneBackend),
        )
        .with_sandbox_limits(crate::sandbox_prefs::SandboxLimits {
            network: Some(false),
            ..Default::default()
        });
        let child = ToolCtx::new(std::env::temp_dir()).inherit_sandbox(&parent);
        // The child's bash must obey the host's forced network policy.
        assert_eq!(child.sandbox_limits().network, Some(false));
        assert!(child.sandbox().is_ok());
        // And it still owns its controllers; nothing was copied wholesale.
        let parent_goal = parent.require::<crate::tools::goal::GoalController>("goal").unwrap();
        let child_goal = child.require::<crate::tools::goal::GoalController>("goal").unwrap();
        assert!(!Arc::ptr_eq(&parent_goal, &child_goal));
    }

    #[test]
    fn extension_keys_by_type_not_by_value() {
        struct Other;
        let mut ext = Extensions::default();
        ext.insert(Arc::new(Marker(1)));
        assert!(ext.get::<Other>().is_none());
    }

    use super::*;

    /// `resolve` must accept a file whose parent directories do not exist yet —
    /// the canonicalize walk climbs to the deepest existing ancestor.
    #[test]
    fn unknown_tool_is_not_readonly() {
        // The turn-start baseline keys off `is_readonly`: a plugin/MCP tool
        // has no built-in spec, so it must count as a potential write.
        let reg = ToolRegistry::with_builtins();
        assert!(!reg.is_readonly("plugin__tool_that_does_not_exist"));
        assert!(reg.is_readonly("smart_read"));
    }

    #[test]
    fn resolve_new_file_in_missing_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(tmp.path());
        let got = ctx.resolve("brand-new/nested/out.docx").unwrap();
        assert_eq!(
            got,
            tmp.path()
                .canonicalize()
                .unwrap()
                .join("brand-new/nested/out.docx")
        );
    }

    /// Paths that escape the workspace still fail even through the walk.
    #[test]
    fn resolve_escape_still_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(tmp.path());
        assert!(ctx.resolve("../../outside.txt").is_err());
    }

    /// A dangling symlink whose target lies outside the workspace must not
    /// resolve: the later write would follow the link out of the sandbox.
    #[cfg(unix)]
    #[test]
    fn resolve_rejects_dangling_symlink_to_outside() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("not-yet.txt");
        std::os::unix::fs::symlink(&target, ws.path().join("link.txt")).unwrap();
        let ctx = ToolCtx::new(ws.path());
        assert!(ctx.resolve("link.txt").is_err());
        assert!(ctx.resolve("link.txt/child").is_err());
        assert!(!target.exists());
    }

    /// A child can read and search, but never persist memory, recurse, or reach
    /// the human. Those are exactly the tools excluded from `child_builtins`.
    #[test]
    fn child_builtins_exclude_persistence_recursion_and_human_tools() {
        let child = ToolRegistry::child_builtins();
        // The goal and plan declarations live only in the parent's goal/plan
        // registries. A child that held them would reach the controller that
        // `delegate` installs for it, so this list also guards that wiring.
        for banned in [
            "remember",
            "delegate",
            "computer",
            "ask_question",
            "goal_complete",
            "goal_blocked",
            "submit_plan",
        ] {
            assert!(
                child.spec(banned).is_none(),
                "child registry must not hold `{banned}`"
            );
        }
        assert!(child.spec("smart_read").is_some());
        assert!(child.spec("bash").is_some());
        // The parent still has the full set, so the exclusion is the child's alone.
        assert!(ToolRegistry::with_builtins().spec("remember").is_some());
    }
}
