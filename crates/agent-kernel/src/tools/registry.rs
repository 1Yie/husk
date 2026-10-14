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

// Host-owned capability slots live in `host_slots`; re-exported so existing
// `crate::tools::registry::X` paths keep working.
pub use crate::tools::host_slots::*;

/// Arguments as delivered by the model — a JSON object.
pub type Args = serde_json::Value;

/// Shared context handed to every tool invocation.



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










pub struct ToolCtx {
    /// Canonical workspace root — all path args resolve and are checked
    /// against this (symlink-escape guard: canonicalize before compare).
    pub workspace_root: Arc<Path>,
    /// Parent turn's cooperative cancel flag — a delegated subagent polls
    /// it so a user cancel propagates into the delegation instead of
    /// orphaning a running child. `None` in unattached contexts.
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
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












    /// A required host service. A context built without it refuses the tool
    /// with a reason instead of panicking mid-turn.
    pub fn require<T: Any + Send + Sync>(&self, what: &str) -> Result<Arc<T>, ToolError> {
        self.ext
            .get::<T>()
            .ok_or_else(|| ToolError::Failed(format!("{what} is not available in this context")))
    }












    /// Same context with a different cancel flag — `batch_execute` scopes
    /// items to a batch deadline flag so a timeout can abort in-flight
    /// blocking work without touching the turn's cancel.
    pub fn with_cancel(&self, cancel: Arc<std::sync::atomic::AtomicBool>) -> ToolCtx {
        ToolCtx {
            workspace_root: self.workspace_root.clone(),
            cancel: Some(cancel),
            active_registry: std::sync::RwLock::new(self.active_registry.read().unwrap().clone()),
            depth: self.depth,
            ext: self.ext.clone(),
        }
    }


    /// Resolve a model-supplied path against the workspace root and prove it
    /// stays inside. Returns the canonical path or a sandbox-escape error.
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
    fn ui_sink_is_absent_until_installed_and_shared_after_cancel_scope() {
        let mut ctx = ToolCtx::new(std::env::temp_dir());
        // Headless by default: fan-out tools skip emitting instead of panicking.
        assert!(ctx.ui().is_none());

        let (sink, _rx) = crate::channels::UiSink::channel();
        ctx.install_ui(sink);
        assert!(ctx.ui().is_some());
        // A batch item's cancel-scoped copy emits on the same sink.
        let scoped = ctx.with_cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        assert!(scoped.ui().is_some());
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
