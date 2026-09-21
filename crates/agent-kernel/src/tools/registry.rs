//! `ToolSpec` + dispatch — one registry for built-ins and plugin tools.
//!
//! Contract (kernel-architecture.md §Tool registry):
//!
//! ```text
//! pub struct ToolSpec {
//!     pub name: &'static str,
//!     pub schema: serde_json::Value,   // JSON Schema for the LLM
//!     pub readonly: bool,             // skips confirmation in `default` mode
//!     pub exec: fn(Args, &ToolCtx) -> BoxFuture<ToolResult>,
//! }
//! ```
//!
//! Plugin tools merge here under `plugin_id:name` (Stage 9). The registry
//! produces the `tools` array for the LLM request and dispatches by name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;

/// Arguments as delivered by the model — a JSON object.
pub type Args = serde_json::Value;

/// Shared context handed to every tool invocation.
#[derive(Clone)]
pub struct ToolCtx {
    /// Canonical workspace root — all path args resolve and are checked
    /// against this (symlink-escape guard: canonicalize before compare).
    pub workspace_root: Arc<Path>,
    /// The sandbox backend for process tools (`bash`, `test_runner`, pty).
    /// `id() == "none"` means loud-unsandboxed: every `bash` must confirm.
    pub sandbox: Arc<dyn agent_sandbox::SandboxBackend>,
    /// Owning session's id + its per-workspace store. Tools that keep
    /// per-session scratch state (`todo`) write next to the session's
    /// history file in the app state dir — never into the user's repo.
    /// `None` for unattached contexts (tests, headless spawns).
    pub session: Option<(i64, Arc<crate::session_store::SessionStore>)>,
    /// Parent turn's cooperative cancel flag — a delegated subagent polls
    /// it so a user cancel propagates into the delegation instead of
    /// orphaning a running child. `None` in unattached contexts.
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Subagent spawner — `delegate` runs a fresh-context child engine.
    /// `None` where delegation is unavailable (tests, child contexts).
    pub subagent: Option<crate::tools::delegate::SubagentSpawner>,
    /// Goal-mode contract signal: `goal_complete`/`goal_blocked` write it,
    /// the engine reads it when the model goes quiet. 0 running · 1 done ·
    /// 2 blocked. Always present (cheap); only consulted in `goal` mode.
    pub goal: Arc<std::sync::atomic::AtomicU8>,
}

impl ToolCtx {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        let (sandbox, _loud) = agent_sandbox::detect_backend();
        Self {
            workspace_root: Arc::from(root.as_path()),
            sandbox: Arc::from(sandbox),
            session: None,
            cancel: None,
            subagent: None,
            goal: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        }
    }

    /// Explicit backend (tests / custom wiring).
    pub fn with_sandbox(root: impl Into<PathBuf>, sandbox: Arc<dyn agent_sandbox::SandboxBackend>) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        Self {
            workspace_root: Arc::from(root.as_path()),
            sandbox,
            session: None,
            cancel: None,
            subagent: None,
            goal: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        }
    }

    /// Attach the owning session — gives tools access to per-session
    /// scratch files in the app state dir.
    pub fn with_session(
        mut self,
        id: i64,
        store: Option<Arc<crate::session_store::SessionStore>>,
    ) -> Self {
        self.session = store.map(|s| (id, s));
        self
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
        // canonicalize the parent and re-attach the filename.
        let canon = match joined.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                let parent = joined
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .ok_or_else(|| ToolError::PathEscape(path.to_string()))?;
                parent.join(joined.file_name().ok_or_else(|| {
                    ToolError::PathEscape(path.to_string())
                })?)
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
        Self { path, content, op: WriteOp::Write, auto_mkdir: false }
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
}

impl ToolResult {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ui_type: None,
            fuzzy: false,
            pending_write: Vec::new(),
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
    dyn Fn(Args, Arc<ToolCtx>) -> BoxFuture<'static, Result<ToolResult, ToolError>>
        + Send
        + Sync,
>;

/// One tool: name, schema for the LLM, readonly flag, exec fn.
#[derive(Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    /// JSON Schema object describing the tool's parameters.
    pub schema: serde_json::Value,
    /// Read-only tools auto-run in `default` permission mode.
    pub readonly: bool,
    pub exec: ExecFn,
}

/// Name → spec map. `BTreeMap` keeps the `tools` array deterministic
/// (provider prompt-cache friendly).
#[derive(Default, Clone)]
pub struct ToolRegistry {
    specs: BTreeMap<&'static str, ToolSpec>,
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
        r.register(crate::tools::todo::spec());
        r.register(crate::tools::serena::spec());
        r.register(crate::tools::web_fetch::spec());
        r.register(crate::tools::web_fetch::spec_alias());
        r.register(crate::tools::delegate::spec());
        r
    }

    pub fn register(&mut self, spec: ToolSpec) {
        self.specs.insert(spec.name, spec);
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
pub fn schema_for<T: schemars::JsonSchema>(
    description: &str,
) -> serde_json::Value {
    let params = schemars::schema_for!(T);
    let params = serde_json::to_value(params).unwrap_or_default();
    serde_json::json!({ "description": description, "parameters": params })
}
