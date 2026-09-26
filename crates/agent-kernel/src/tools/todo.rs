//! `todo` — session-scoped task list the agent maintains while it works.
//!
//! Stored beside the session's history in `sessions.db`
//! (`state/<ws>/<id>/todos.json` row): a new session starts clean, a resumed
//! one keeps its list, and the user's repo stays free of agent scratch files.
//! That is also why `readonly: true` is honest — it never touches the
//! workspace, so it needs no approval.
//!
//! Two invariants: one load→mutate→save per store row at a time
//! (`state_lock`), and ids are never reused, not even across `clear` — a
//! recycled `#1` makes the model mark the wrong item done.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};

use futures::FutureExt;
use serde::{Deserialize, Serialize};

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};
use crate::session_store::SessionStore;

/// One todo entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: usize,
    pub text: String,
    pub done: bool,
}

/// Persisted store — the `state/<ws>/<id>/todos.json` row.
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
            "Manage this session's task list (persisted with the session).\n\
             Actions: `add` a task (`text`), `list` all tasks, `done`/`undone`\n\
             (`id`), `remove` (`id`), `clear` (wipe the list). Use it to track\n\
             multi-step work the user can see — plan, subtasks, follow-ups.",
        ),
        readonly: true,
        class: super::registry::ToolClass::SessionMutation,
        network: false,
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

fn store_handle(ctx: &ToolCtx) -> Option<(Arc<SessionStore>, i64)> {
    if let Some((id, store)) = &ctx.session {
        return Some((store.clone(), *id));
    }
    // Unattached context (tests/headless): derive the workspace's session
    // store anyway so state still lives in the app data dir.
    crate::session_store::SessionStore::open(&ctx.workspace_root)
        .ok()
        .map(|s| (Arc::new(s), 0))
}

/// One async lock per store row, shared process-wide.
///
/// `exec` is a read-modify-write cycle, so two overlapping invocations can
/// lose an update or hand the same id to two items. Today the engine
/// dispatches a round's calls sequentially — this is insurance, not a fix for
/// a reproducible bug — but the failure is *silent* (a todo just vanishes),
/// and parallel dispatch is the direction the runtime is heading. Keyed by
/// store key rather than held in `ToolCtx` so it also covers a second
/// `ToolCtx` over the same session (subagent, headless context).
fn state_lock(key: String) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let locks = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(key).or_default().clone()
}

fn lock_key(store: &SessionStore, id: i64) -> String {
    format!("{}:{}:todos", store.dir_name().unwrap_or_default(), id)
}

async fn load(
    store: &SessionStore,
    id: i64,
    workspace_root: &Path,
) -> Result<TodoStore, ToolError> {
    match store.read_state(id, "todos.json") {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes)
            .map_err(|e| ToolError::Failed(format!("corrupt todos row for session {id}: {e}"))),
        Ok(None) => {
            // One-time migration: a legacy `{workspace}/.agent/todos.json`
            // becomes this session's list, then the repo file is removed so
            // the workspace stays clean.
            let legacy = workspace_root.join(".agent").join("todos.json");
            match tokio::fs::read_to_string(&legacy).await {
                Ok(s) => {
                    // Unreadable legacy state is NOT an empty list: treating
                    // it as one would strand the user's tasks *and* the file
                    // is deleted a line later. Surface it and leave it alone.
                    let seeded: TodoStore = serde_json::from_str(&s).map_err(|e| {
                        ToolError::Failed(format!(
                            "legacy {} is not valid todo JSON: {e} — left in place; \
                             fix or delete it and retry",
                            legacy.display()
                        ))
                    })?;
                    // Copy first, remove only after the new copy is durable —
                    // a failed migration must not be a destructive one.
                    save(store, id, &seeded)?;
                    let _ = tokio::fs::remove_file(&legacy).await;
                    Ok(seeded)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TodoStore {
                    next_id: 1,
                    items: Vec::new(),
                }),
                Err(e) => Err(ToolError::Io(e)),
            }
        }
        Err(e) => Err(ToolError::Io(e)),
    }
}

/// Single-row put — the redb commit IS the atomic replace the old
/// temp-file+rename dance simulated, so a crash shows either the previous
/// row or the new one, never a truncated file.
fn save(store: &SessionStore, id: i64, todos: &TodoStore) -> Result<(), ToolError> {
    let json = serde_json::to_vec_pretty(todos)
        .map_err(|e| ToolError::Failed(format!("serialize todos: {e}")))?;
    store
        .write_state(id, "todos.json", &json)
        .map_err(ToolError::Io)
}

/// Render the list as a compact `[ ]/[x] #id text` block for the model.
fn render(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "No todos.".into();
    }
    let done = items.iter().filter(|t| t.done).count();
    let mut s = format!("{}/{} done\n", done, items.len());
    for t in items {
        s.push_str(&format!(
            "[{}] #{} {}\n",
            if t.done { "x" } else { " " },
            t.id,
            t.text
        ));
    }
    s.trim_end().to_string()
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let a: TodoArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(format!("todo args: {e}")))?;
    let (sess, sid) = store_handle(&ctx)
        .ok_or_else(|| ToolError::Failed("no data dir for the todo store".into()))?;
    let lock = state_lock(lock_key(&sess, sid));
    let _guard = lock.lock().await;
    let mut store = load(&sess, sid, &ctx.workspace_root).await?;
    // `next_id` must clear the highest stored id — a file with `next_id: 2`
    // sitting next to `#10` (hand-edited, partially migrated, older format)
    // would otherwise hand out a duplicate id.
    let max_id = store.items.iter().map(|t| t.id).max().unwrap_or(0);
    store.next_id = store.next_id.max(max_id + 1);

    let out = match a.action.as_str() {
        "list" => render(&store.items),
        "add" => {
            let text = a
                .text
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| ToolError::Args("`add` needs a non-empty `text`".into()))?;
            let item = TodoItem {
                id: store.next_id,
                text: text.into(),
                done: false,
            };
            store.next_id += 1;
            store.items.push(item.clone());
            save(&sess, sid, &store)?;
            format!(
                "Added #{} {}\n\n{}",
                item.id,
                item.text,
                render(&store.items)
            )
        }
        "done" | "undone" => {
            let id =
                a.id.ok_or_else(|| ToolError::Args(format!("`{}` needs an `id`", a.action)))?;
            let item = store.items.iter_mut().find(|t| t.id == id).ok_or_else(|| {
                ToolError::Failed(format!("todo #{id} not found — run `list` for live ids"))
            })?;
            item.done = a.action == "done";
            let done = item.done;
            save(&sess, sid, &store)?;
            format!(
                "#{} {}\n\n{}",
                id,
                if done { "done" } else { "reopened" },
                render(&store.items)
            )
        }
        "remove" => {
            let id =
                a.id.ok_or_else(|| ToolError::Args("`remove` needs an `id`".into()))?;
            let before = store.items.len();
            store.items.retain(|t| t.id != id);
            if store.items.len() == before {
                return Err(ToolError::Failed(format!(
                    "todo #{id} not found — run `list` for live ids"
                )));
            }
            save(&sess, sid, &store)?;
            format!("Removed #{id}\n\n{}", render(&store.items))
        }
        "clear" => {
            let n = store.items.len();
            store.items.clear();
            // `next_id` deliberately survives a clear — see the module docs.
            save(&sess, sid, &store)?;
            format!("Cleared {n} todos.")
        }
        other => {
            return Err(ToolError::Args(format!(
                "unknown action '{other}' — use add|list|done|undone|remove|clear"
            )))
        }
    };

    Ok(ToolResult {
        content: out,
        ui_type: Some("todo"),
        fuzzy: false,
        pending_write: Vec::new(),
        images: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Arc<ToolCtx> {
        // Unique dir per test — sharing one across parallel tests
        // cross-pollutes the per-session store row and flakes the roundtrip.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "todo-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Attach an isolated store (its own db inside the temp dir) so the
        // tests never touch the real `sessions.db`.
        let store = SessionStore::open_at(&dir, &dir.join("t.db")).unwrap();
        let mut c = ToolCtx::new(&dir);
        c.session = Some((0, Arc::new(store)));
        Arc::new(c)
    }

    #[tokio::test]
    async fn add_list_done_remove_roundtrip() {
        let ctx = ctx();
        let r = exec(
            serde_json::json!({"action":"add","text":"fix the bug"}),
            ctx.clone(),
        )
        .await
        .unwrap();
        assert!(r.content.contains("#1"));

        let r = exec(serde_json::json!({"action":"list"}), ctx.clone())
            .await
            .unwrap();
        assert!(r.content.contains("fix the bug"));

        let r = exec(serde_json::json!({"action":"done","id":1}), ctx.clone())
            .await
            .unwrap();
        assert!(r.content.contains("[x]"));

        let r = exec(serde_json::json!({"action":"remove","id":1}), ctx.clone())
            .await
            .unwrap();
        assert!(r.content.contains("Removed #1"));

        // persisted across "sessions" (fresh load from the same store)
        let (sess, sid) = store_handle(&ctx).unwrap();
        let store = load(&sess, sid, &ctx.workspace_root).await.unwrap();
        assert!(store.items.is_empty());
    }

    /// Ids never recycle — a fresh `#1` for a different task aliases with the
    /// `#1` still in the model's context window (hence: monotonic across
    /// `clear`, and derived from the highest stored id on load).
    #[tokio::test]
    async fn ids_stay_monotonic_across_clear() {
        let ctx = ctx();
        exec(serde_json::json!({"action":"add","text":"a"}), ctx.clone())
            .await
            .unwrap();
        exec(serde_json::json!({"action":"add","text":"b"}), ctx.clone())
            .await
            .unwrap();
        let r = exec(serde_json::json!({"action":"clear"}), ctx.clone())
            .await
            .unwrap();
        assert!(r.content.contains("Cleared 2"));
        let r = exec(serde_json::json!({"action":"add","text":"c"}), ctx.clone())
            .await
            .unwrap();
        assert!(
            r.content.contains("#3"),
            "id recycled after clear: {}",
            r.content
        );
    }

    /// A hand-edited / older file can carry a `next_id` below the highest id.
    #[tokio::test]
    async fn next_id_clears_the_highest_stored_id() {
        let ctx = ctx();
        let (sess, sid) = store_handle(&ctx).unwrap();
        let seeded = serde_json::json!({
            "next_id": 2,
            "items": [{"id": 10, "text": "old task", "done": false}],
        });
        sess.write_state(
            sid,
            "todos.json",
            serde_json::to_string(&seeded).unwrap().as_bytes(),
        )
        .unwrap();

        let r = exec(
            serde_json::json!({"action":"add","text":"new"}),
            ctx.clone(),
        )
        .await
        .unwrap();
        assert!(
            r.content.contains("#11"),
            "duplicate id handed out: {}",
            r.content
        );
    }

    /// Overlapping invocations must not lose an add. Without the per-store
    /// lock this drops updates: every call would load the same snapshot and
    /// the last save would win.
    #[tokio::test]
    async fn concurrent_adds_neither_collide_nor_vanish() {
        let ctx = ctx();
        let mut set = tokio::task::JoinSet::new();
        for i in 0..8 {
            let ctx = ctx.clone();
            set.spawn(async move {
                exec(
                    serde_json::json!({"action":"add","text":format!("t{i}")}),
                    ctx,
                )
                .await
                .unwrap()
            });
        }
        while set.join_next().await.is_some() {}

        let (sess, sid) = store_handle(&ctx).unwrap();
        let store = load(&sess, sid, &ctx.workspace_root).await.unwrap();
        assert_eq!(store.items.len(), 8, "lost update: {:?}", store.items);
        let mut ids: Vec<usize> = store.items.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=8).collect::<Vec<_>>(), "id collision: {ids:?}");
    }

    /// Corrupt legacy state must not be mistaken for "no todos" — that both
    /// strands the user's tasks and (before this) deleted the only copy.
    #[tokio::test]
    async fn corrupt_legacy_migration_fails_loudly_and_keeps_the_file() {
        let ctx = ctx();
        let legacy = ctx.workspace_root.join(".agent").join("todos.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{ this is not json").unwrap();

        let err = exec(serde_json::json!({"action":"list"}), ctx.clone())
            .await
            .expect_err("a corrupt legacy file must surface, not read as empty");
        assert!(
            matches!(err, ToolError::Failed(ref m) if m.contains("legacy")),
            "{err}"
        );
        assert!(legacy.exists(), "the only copy of the list was deleted");
    }

    #[tokio::test]
    async fn legacy_migration_copies_before_removing() {
        let ctx = ctx();
        let legacy = ctx.workspace_root.join(".agent").join("todos.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let seeded = serde_json::json!({
            "next_id": 4,
            "items": [{"id": 3, "text": "old task", "done": true}],
        });
        std::fs::write(&legacy, serde_json::to_string(&seeded).unwrap()).unwrap();

        let r = exec(serde_json::json!({"action":"list"}), ctx.clone())
            .await
            .unwrap();
        assert!(r.content.contains("#3 old task"), "{}", r.content);
        assert!(
            !legacy.exists(),
            "migration should clean the workspace copy"
        );
        // …and the new home holds it (ids included).
        let r = exec(
            serde_json::json!({"action":"add","text":"next"}),
            ctx.clone(),
        )
        .await
        .unwrap();
        assert!(r.content.contains("#4 next"), "{}", r.content);
    }

    /// `save` is one atomic commit — the row parses back and re-saving works.
    #[tokio::test]
    async fn save_commits_one_row_and_roundtrips() {
        let ctx = ctx();

        exec(serde_json::json!({"action":"add","text":"a"}), ctx.clone())
            .await
            .unwrap();

        let (sess, sid) = store_handle(&ctx).unwrap();
        let raw = sess.read_state(sid, "todos.json").unwrap().unwrap();
        let store: TodoStore = serde_json::from_slice(&raw).unwrap();
        assert_eq!(store.items.len(), 1);
    }

    #[tokio::test]
    async fn ids_survive_restart() {
        let ctx = ctx();
        exec(serde_json::json!({"action":"add","text":"a"}), ctx.clone())
            .await
            .unwrap();
        exec(serde_json::json!({"action":"add","text":"b"}), ctx.clone())
            .await
            .unwrap();
        // Fresh load — next_id must keep climbing, not reset to 1.
        let r = exec(serde_json::json!({"action":"add","text":"c"}), ctx.clone())
            .await
            .unwrap();
        assert!(
            r.content.contains("#3"),
            "ids collided after reload: {}",
            r.content
        );
    }
}
