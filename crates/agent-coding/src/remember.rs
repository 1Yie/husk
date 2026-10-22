//! `remember` / `memory_search` — the agent's write and read paths into
//! persistent memory.
//!
//! Writing lets the agent decide in the moment that something the user said
//! is a durable preference rather than a one-off request — "以后都用中文回答"
//! is worth remembering, "改一下这个文件" is not. Entries land in the shared
//! persona k/v of `memory.db`; the host injects what the session has not seen
//! yet on the next message, and every new session renders it up-front. The
//! user's `/remember` + `/forget` commands and the memory distiller write the
//! same table.
//!
//! Reading exists because the system prompt only carries persona: facts are
//! pushed per message and episodes not at all, so an explicit question about
//! what was decided earlier needs a tool.

use std::sync::Arc;

use futures::FutureExt;

use agent_kernel::tools::registry::{
    schema_for, ExecFn, ToolClass, ToolError, ToolResult, ToolSpec,
};

use agent_context::memory::{MemoryStore, PersonaScope};

/// Shared, swappable handle to the workspace memory store. Cloning shares the
/// same slot, so a subagent or a tool sees the owning actor's current switch
/// state rather than a snapshot from spawn time. Installed on the context as a
/// shared slot ([`install`](Self::install)) so a delegated child's `remember`
/// lands in the same workspace store.
#[derive(Clone, Default)]
pub struct MemoryHandle(Arc<std::sync::RwLock<Option<Arc<MemoryStore>>>>);

impl MemoryHandle {
    /// The store as of now — `None` while memory is switched off.
    pub fn get(&self) -> Option<Arc<MemoryStore>> {
        self.0.read().ok().and_then(|g| g.clone())
    }

    /// Replace the store (`None` switches memory off).
    pub fn set(&self, store: Option<Arc<MemoryStore>>) {
        if let Ok(mut g) = self.0.write() {
            *g = store;
        }
    }

    /// Install this handle on a context, shared with delegated children.
    pub fn install(&self, ctx: &mut agent_kernel::tools::registry::ToolCtx) {
        ctx.ext.insert_shared(Arc::new(self.clone()));
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct RememberArgs {
    /// Short label — `language`, `style`, `test_cmd`, … Reusing a key
    /// overwrites its value.
    key: String,
    /// The durable preference to keep — one line.
    value: String,
    /// `true` when the preference should follow the user into every project
    /// (回答语言、语气、提交习惯). Default `false` — this workspace only.
    #[serde(default)]
    global: bool,
}

pub fn spec() -> ToolSpec {
    let exec: ExecFn = Arc::new(|args, ctx| {
        let parsed: RememberArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return async move { Err(ToolError::Args(format!("remember args: {e}"))) }.boxed()
            }
        };
        async move {
            let Some(memory) = ctx.ext.get::<MemoryHandle>().and_then(|m| m.get()) else {
                return Err(ToolError::Failed(
                    // `None` = the session spawned without a store: the
                    // memory toggle is off, or the db failed to open.
                    "remember is unavailable — memory disabled or store failed to open".into(),
                ));
            };
            let key = parsed.key.trim();
            let value = parsed.value.trim();
            if key.is_empty() || value.is_empty() {
                return Err(ToolError::Args(
                    "`remember` needs a non-empty `key` and `value`".into(),
                ));
            }
            // Memory is injected into every later system prompt, so a pasted
            // credential would travel with every request. Only the value is
            // checked: a key is a user-facing label, and the name denylist
            // would wrongly refuse a legitimate label like `toggle_on_key`.
            if agent_context::memory::distill::looks_secret(value) {
                return Err(ToolError::Args(
                    "refused: this looks like a credential — memory keeps preferences \
                     and project facts, never secrets. Store the reference (e.g. the \
                     env var name) instead."
                        .into(),
                ));
            }
            let scope = if parsed.global {
                PersonaScope::Global
            } else {
                PersonaScope::Workspace
            };
            // The write opens the store's only write transaction and, with the
            // semantic model loaded, runs inference before it — seconds of CPU
            // that must not hold a runtime worker every session shares.
            let key = key.to_string();
            let value = value.to_string();
            let written = value.clone();
            let landed = tokio::task::spawn_blocking(move || {
                memory.set_persona_scoped(scope, &key, &written)
            })
            .await
            .map_err(|e| ToolError::Failed(format!("memory write failed: {e}")))?
            .map_err(|e| ToolError::Failed(format!("memory write failed: {e}")))?;
            let where_ = if parsed.global {
                "all workspaces"
            } else {
                "this workspace"
            };
            Ok(ToolResult::text(format!(
                "remembered `{landed}: {value}` ({where_})"
            )))
        }
        .boxed()
    });
    ToolSpec {
        name: "remember",
        schema: schema_for::<RememberArgs>(
            "Store a durable user preference in persistent memory — the user sees \
             it from the next message on, and every new session starts with it. \
             Call it when the user states something meant to hold across sessions \
             (\"用中文回答\", \"先跑 fmt 再 test\"); a one-off request is NOT memory. \
             `key` is a short label (reuse overwrites), `value` one line, and \
             `global: true` only for preferences that should follow the user into \
             other projects — everything else stays scoped to this workspace. \
             The user can see entries via `/memory` and remove them via \
             `/forget <key>` — never store secrets or ephemeral task state.",
        ),
        readonly: false,
        class: ToolClass::SessionMutation,
        network: false,
        staged: false,
        review: None,
        exec,
    }
}

/// What the agent asks memory for.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    /// The question or topic to look up — natural language, not a regex.
    query: String,
    /// How many stored facts to consider (default 5).
    limit: Option<usize>,
}

/// `memory_search` — the read side of the same store. Read-only: the system
/// prompt carries persona, facts arrive per message, and episodes not at all,
/// so "what did we decide about X" needs a way to ask.
pub fn search_spec() -> ToolSpec {
    let exec: ExecFn = Arc::new(|args, ctx| {
        let parsed: SearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return async move { Err(ToolError::Args(format!("memory_search args: {e}"))) }
                    .boxed()
            }
        };
        async move {
            let Some(memory) = ctx.ext.get::<MemoryHandle>().and_then(|m| m.get()) else {
                return Err(ToolError::Failed(
                    "memory_search is unavailable — memory disabled or store failed to open".into(),
                ));
            };
            let query = parsed.query.trim();
            if query.is_empty() {
                return Err(ToolError::Args(
                    "`memory_search` needs a non-empty query".into(),
                ));
            }
            // Recall scans the fact table and, with the semantic model loaded,
            // runs inference — seconds of CPU that must not hold a runtime
            // worker every session shares.
            let query = query.to_string();
            let limit = parsed.limit.unwrap_or(5);
            let report = tokio::task::spawn_blocking(move || memory.search_report(&query, limit))
                .await
                .map_err(|e| ToolError::Failed(format!("memory read failed: {e}")))?
                .map_err(|e| ToolError::Failed(format!("memory read failed: {e}")))?;
            Ok(ToolResult::text(report))
        }
        .boxed()
    });
    ToolSpec {
        name: "memory_search",
        schema: schema_for::<SearchArgs>(
            "Search persistent project memory — facts distilled from earlier \
             sessions plus the most recent turn summaries. The system prompt \
             already carries the user's stated preferences; call this for project \
             knowledge (build/test commands, conventions, past decisions) that is \
             not already in front of you. An empty answer means nothing was ever \
             stored — it is not an error.",
        ),
        readonly: true,
        class: ToolClass::Observation,
        network: false,
        staged: false,
        review: None,
        exec,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_capabilities::WorkspaceCtx;
    use agent_kernel::tools::registry::ToolCtx;

    /// A delegated child shares the very same handle, so a later memory switch
    /// on the parent's side is visible to the child without re-inheriting — and
    /// an uninstalled context simply has none.
    #[test]
    fn a_child_sees_the_parents_live_memory_switch() {
        let mut parent = ToolCtx::new().with_workspace(std::env::temp_dir());
        assert!(parent.ext.get::<MemoryHandle>().is_none());

        let handle = MemoryHandle::default();
        handle.install(&mut parent);
        // Installed but switched off: the slot exists, the store is None.
        assert!(parent.ext.get::<MemoryHandle>().unwrap().get().is_none());

        let mut child = ToolCtx::new().with_workspace(std::env::temp_dir());
        child.ext = parent.ext.inheritable();
        let theirs = child
            .ext
            .get::<MemoryHandle>()
            .expect("child inherits memory");
        assert!(
            Arc::ptr_eq(&theirs.0, &handle.0),
            "same slot, not a snapshot"
        );
    }

    /// `remember` refuses cleanly when memory is off rather than panicking.
    #[tokio::test]
    async fn remember_refuses_when_memory_is_off() {
        let mut ctx = ToolCtx::new().with_workspace(std::env::temp_dir());
        MemoryHandle::default().install(&mut ctx);
        let spec = spec();
        let err = (spec.exec)(serde_json::json!({"key": "k", "value": "v"}), Arc::new(ctx))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("remember is unavailable"), "{err}");
    }
}
