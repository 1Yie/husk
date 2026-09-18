//! `todo` — a persistent task list the agent manages while it works.
//!
//! Mirrors the pi-todo extension's model (list/add/toggle/clear) but stores
//! state in `{workspace}/.agent/todos.json` instead of tool-result snapshots
//! — agent-rs has no session-branch `details` mechanism, and a workspace
//! file keeps the list stable across sessions and restarts, which matches
//! how a human would expect a project todo list to behave.
//!
//! `readonly: false` — it mutates state — but it is NOT a write tool in the
//! `PendingWrite` sense: it touches `.agent/` metadata, never source files,
//! so it bypasses the diff/approval flow and commits its own side effect
//! inside `exec` (same as `bash`'s managed side effects).

use std::sync::Arc;

use futures::FutureExt;
use serde::{Deserialize, Serialize};

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// One todo entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: usize,
    pub text: String,
    pub done: bool,
}

/// On-disk store — `{workspace}/.agent/todos.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TodoStore {
    next_id: usize,
    items: Vec<TodoItem>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TodoArgs {
    /// What to do: `add` (needs `text`), `list`, `done`/`undone`/`remove`
    /// (need `id`), `clear` (wipes the list).
    action: String,
    /// Todo text — required for `add`.
    text: Option<String>,
    /// Todo id — required for `done`/`undone`/`remove`.
    id: Option<usize>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "todo",
        schema: schema_for::<TodoArgs>(
            "Manage a persistent project todo list (stored in `.agent/todos.json`).\n\
             Actions: `add` a task (`text`), `list` all tasks, `done`/`undone`\n\
             (`id`), `remove` (`id`), `clear` (wipe the list). Use it to track\n\
             multi-step work the user can see — plan, subtasks, follow-ups.",
        ),
        readonly: false,
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

fn store_path(ctx: &ToolCtx) -> std::path::PathBuf {
    ctx.workspace_root.join(".agent").join("todos.json")
}

async fn load(ctx: &ToolCtx) -> Result<TodoStore, ToolError> {
    let path = store_path(ctx);
    match tokio::fs::read_to_string(&path).await {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| ToolError::Failed(format!("corrupt {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TodoStore {
            next_id: 1,
            items: Vec::new(),
        }),
        Err(e) => Err(ToolError::Io(e)),
    }
}

async fn save(ctx: &ToolCtx, store: &TodoStore) -> Result<(), ToolError> {
    let path = store_path(ctx);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| ToolError::Failed(format!("serialize todos: {e}")))?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

/// Render the list as a compact `[ ]/[x] #id text` block for the model.
fn render(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "No todos.".into();
    }
    let done = items.iter().filter(|t| t.done).count();
    let mut s = format!("{}/{} done\n", done, items.len());
    for t in items {
        s.push_str(&format!("[{}] #{} {}\n", if t.done { "x" } else { " " }, t.id, t.text));
    }
    s.trim_end().to_string()
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let a: TodoArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("todo args: {e}")))?;
    let mut store = load(&ctx).await?;
    // `next_id` defaults to 0 on a fresh deserialize — pin it past the max
    // id so a hand-edited or first-run file can't collide.
    if store.next_id == 0 {
        store.next_id = store.items.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    }

    let out = match a.action.as_str() {
        "list" => render(&store.items),
        "add" => {
            let text = a
                .text
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| ToolError::Args("`add` needs a non-empty `text`".into()))?;
            let item = TodoItem { id: store.next_id, text: text.into(), done: false };
            store.next_id += 1;
            store.items.push(item.clone());
            save(&ctx, &store).await?;
            format!("Added #{} {}\n\n{}", item.id, item.text, render(&store.items))
        }
        "done" | "undone" => {
            let id = a
                .id
                .ok_or_else(|| ToolError::Args(format!("`{}` needs an `id`", a.action)))?;
            let item = store
                .items
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| {
                    ToolError::Failed(format!(
                        "todo #{id} not found — run `list` for live ids"
                    ))
                })?;
            item.done = a.action == "done";
            let done = item.done;
            save(&ctx, &store).await?;
            format!("#{} {}\n\n{}", id, if done { "done" } else { "reopened" }, render(&store.items))
        }
        "remove" => {
            let id = a.id.ok_or_else(|| ToolError::Args("`remove` needs an `id`".into()))?;
            let before = store.items.len();
            store.items.retain(|t| t.id != id);
            if store.items.len() == before {
                return Err(ToolError::Failed(format!(
                    "todo #{id} not found — run `list` for live ids"
                )));
            }
            save(&ctx, &store).await?;
            format!("Removed #{id}\n\n{}", render(&store.items))
        }
        "clear" => {
            let n = store.items.len();
            store.items.clear();
            store.next_id = 1;
            save(&ctx, &store).await?;
            format!("Cleared {n} todos.")
        }
        other => {
            return Err(ToolError::Args(format!(
                "unknown action '{other}' — use add|list|done|undone|remove|clear"
            )))
        }
    };

    Ok(ToolResult { content: out, ui_type: Some("todo"), fuzzy: false, pending_write: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Arc<ToolCtx> {
        // Unique dir per test — sharing one path across parallel tests
        // cross-pollutes `.agent/todos.json` and flakes the roundtrip.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "todo-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(ToolCtx::new(&dir))
    }

    #[tokio::test]
    async fn add_list_done_remove_roundtrip() {
        let ctx = ctx();
        // add
        let r = exec(
            serde_json::json!({"action":"add","text":"fix the bug"}),
            ctx.clone(),
        )
        .await
        .unwrap();
        assert!(r.content.contains("#1"));

        // list
        let r = exec(serde_json::json!({"action":"list"}), ctx.clone()).await.unwrap();
        assert!(r.content.contains("fix the bug"));

        // done
        let r = exec(serde_json::json!({"action":"done","id":1}), ctx.clone()).await.unwrap();
        assert!(r.content.contains("[x]"));

        // remove
        let r = exec(serde_json::json!({"action":"remove","id":1}), ctx.clone()).await.unwrap();
        assert!(r.content.contains("Removed #1"));

        // persisted across "sessions" (fresh load from the same dir)
        let store = load(&ctx).await.unwrap();
        assert!(store.items.is_empty());
    }

    #[tokio::test]
    async fn ids_survive_restart() {
        let ctx = ctx();
        exec(serde_json::json!({"action":"add","text":"a"}), ctx.clone()).await.unwrap();
        exec(serde_json::json!({"action":"add","text":"b"}), ctx.clone()).await.unwrap();
        // Fresh load — next_id must keep climbing, not reset to 1.
        let r = exec(serde_json::json!({"action":"add","text":"c"}), ctx.clone()).await.unwrap();
        assert!(r.content.contains("#3"), "ids collided after reload: {}", r.content);
    }
}
