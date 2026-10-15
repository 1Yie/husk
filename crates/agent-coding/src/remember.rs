//! `remember` — the agent's write path into persistent memory.
//!
//! The read side already exists (the `<memory>` block in the system prompt);
//! this tool lets the agent decide in the moment that something the user
//! said is a durable preference rather than a one-off request — "以后都用
//! 中文回答" is worth remembering, "改一下这个文件" is not. Entries land in
//! the shared persona k/v of `memory.db` and surface in the `── persona ──`
//! section from the next turn on. The user's `/remember` + `/forget`
//! commands and the memory distiller write the same table.

use std::sync::Arc;

use futures::FutureExt;

use agent_kernel::tools::registry::{
    schema_for, ExecFn, ToolClass, ToolError, ToolResult, ToolSpec,
};

use agent_context::memory::MemoryStore;

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
            memory
                .set_persona(key, value)
                .map_err(|e| ToolError::Failed(format!("memory write failed: {e}")))?;
            Ok(ToolResult::text(format!("remembered `{key}: {value}`")))
        }
        .boxed()
    });
    ToolSpec {
        name: "remember",
        schema: schema_for::<RememberArgs>(
            "Store a durable user preference in persistent memory — it surfaces \
             in the `── persona ──` block every following turn. Call it when the \
             user states something meant to hold across sessions (\"用中文回答\", \
             \"先跑 fmt 再 test\"); a one-off request is NOT memory. `key` is a \
             short label (reuse overwrites), `value` one line. The user can see \
             entries via `/memory` and remove them via `/forget <key>` — never \
             store secrets or ephemeral task state.",
        ),
        readonly: false,
        class: ToolClass::SessionMutation,
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
