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

use super::registry::{schema_for, ExecFn, ToolClass, ToolError, ToolResult, ToolSpec};

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
            let Some(memory) = ctx.memory.clone() else {
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
        exec,
    }
}
