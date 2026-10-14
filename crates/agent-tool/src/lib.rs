//! The tool contract: what a tool declares, what it returns, and the error it
//! raises. Nothing here knows about a product.
//!
//! [`ToolSpec`] is generic over the context type `C` so this crate does not
//! name the host's context; the kernel instantiates it with its own `ToolCtx`.
//! Classes, results, deferred writes, errors and the argument/schema helpers
//! are context-free.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;

/// Arguments as delivered by the model — a JSON object.
pub type Args = serde_json::Value;

/// Typed extension slots for a tool context. A tool that needs a host-owned
/// service reads it by type, so the context itself carries no product fields
/// and this crate never has to name the types that provide them.
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

/// What a workspace-scoped tool needs from the context it runs in.
///
/// The kernel's `ToolCtx` implements this; the tools themselves only name the
/// trait, so a tool crate can be written, tested and shipped without depending
/// on the kernel that drives it. `root`/`cancel_flag` are accessors rather than
/// fields because a generic `C` has no fields to reach for.
pub trait WorkspaceCtx: Send + Sync + 'static {
    /// Canonical workspace root — path args resolve against this.
    fn root(&self) -> &std::path::Path;

    /// The parent turn's cooperative cancel flag, if this context has one.
    fn cancel_flag(&self) -> Option<Arc<std::sync::atomic::AtomicBool>>;

    /// Resolve a model-supplied path against the workspace root and prove it
    /// stays inside. Returns the canonical path or a sandbox-escape error.
    fn resolve(&self, path: &str) -> Result<std::path::PathBuf, ToolError>;
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
pub type ExecFn<C> = Arc<
    dyn Fn(Args, Arc<C>) -> BoxFuture<'static, Result<ToolResult, ToolError>> + Send + Sync,
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
pub struct ToolSpec<C> {
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
    pub exec: ExecFn<C>,
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
    use super::*;

    struct Marker(u32);

    /// The extension table is the contract's own storage; it must work with no
    /// kernel, no context and no capability types in sight.
    #[test]
    fn extensions_round_trip_without_a_context() {
        let mut ext = Extensions::default();
        assert!(ext.get::<Marker>().is_none());
        ext.insert(Arc::new(Marker(7)));
        assert_eq!(ext.get::<Marker>().map(|m| m.0), Some(7));
    }

    /// A spec is generic over the context, so a product that has no kernel
    /// `ToolCtx` can still declare tools.
    #[test]
    fn a_spec_can_be_built_for_any_context() {
        struct Host;
        let spec: ToolSpec<Host> = ToolSpec {
            name: "demo",
            schema: serde_json::json!({"type": "object"}),
            readonly: true,
            class: ToolClass::Observation,
            network: false,
            exec: Arc::new(|_args: Args, _ctx: Arc<Host>| {
                Box::pin(async move { Ok(ToolResult::text("ok")) })
            }),
        };
        assert_eq!(spec.name, "demo");
        assert_eq!(spec.class, ToolClass::Observation);
        assert!(spec.readonly);
    }

    /// The dry-run set is exact: it is what the engine dispatches before the
    /// permission gate, so a non-writer must never appear here.
    #[test]
    fn the_staged_write_set_is_exact() {
        assert!(is_staged_write("fuzzy_patch"));
        assert!(is_staged_write("apply_patch"));
        assert!(!is_staged_write("serena"), "serena writes in-process");
        assert!(!is_staged_write("write_file"), "no such tool");
        assert!(!is_staged_write("fs_patch"), "module name, not a tool");
    }
}
