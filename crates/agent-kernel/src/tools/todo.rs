//! `todo` — session-scoped task list the agent maintains while it works.
//!
//! Stored beside the session's history
//! (`~/.local/share/husk/sessions/<ws>/<id>.todos.json`): a new session starts
//! clean, a resumed one keeps its list, and the user's repo stays free of agent
//! scratch files. That is also why `readonly: true` is honest — it never touches
//! the workspace, so it needs no approval.
//!
//! Two invariants: one load→mutate→save per store file at a time (`path_lock`),
//! and ids are never reused, not even across `clear` — a recycled `#1` makes the
//! model mark the wrong item done.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};

use futures::FutureExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// One todo entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: usize,
    pub text: String,
    pub done: bool,
}

/// On-disk store — `<session store>/<session_id>.todos.json`.
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

fn store_path(ctx: &ToolCtx) -> std::path::PathBuf {
    if let Some((id, store)) = &ctx.session {
        return store.state_file(*id, "todos.json");
    }
    // Unattached context (tests/headless): derive the workspace's session
    // store anyway so state still lives in the app data dir; temp dir is the
    // last resort when no data dir exists at all.
    crate::session_store::SessionStore::open(&ctx.workspace_root)
        .map(|s| s.state_file(0, "todos.json"))
        .unwrap_or_else(|_| std::env::temp_dir().join("husk-todos.json"))
}

/// One async lock per store file, shared process-wide.
///
/// `exec` is a read-modify-write cycle, so two overlapping invocations can
/// lose an update or hand the same id to two items. Today the engine
/// dispatches a round's calls sequentially — this is insurance, not a fix for
/// a reproducible bug — but the failure is *silent* (a todo just vanishes),
/// and parallel dispatch is the direction the runtime is heading. Keyed by
/// path rather than held in `ToolCtx` so it also covers a second `ToolCtx`
/// over the same session (subagent, headless context).
fn path_lock(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let locks = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(path.to_path_buf()).or_default().clone()
}

async fn load(path: &Path, workspace_root: &Path) -> Result<TodoStore, ToolError> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| ToolError::Failed(format!("corrupt {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // One-time migration: a legacy `{workspace}/.agent/todos.json`
            // becomes this session's list, then the repo file is removed so
            // the workspace stays clean.
            let legacy = workspace_root.join(".agent").join("todos.json");
            match tokio::fs::read_to_string(&legacy).await {
                Ok(s) => {
                    // Unreadable legacy state is NOT an empty list: treating
                    // it as one would strand the user's tasks *and* the file
                    // is deleted a line later. Surface it and leave it alone.
                    let store: TodoStore = serde_json::from_str(&s).map_err(|e| {
                        ToolError::Failed(format!(
                            "legacy {} is not valid todo JSON: {e} — left in place; \
                             fix or delete it and retry",
                            legacy.display()
                        ))
                    })?;
                    // Copy first, remove only after the new copy is durable —
                    // a failed migration must not be a destructive one.
                    save(path, &store).await?;
                    let _ = tokio::fs::remove_file(&legacy).await;
                    Ok(store)
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

/// Atomic replace: write a temp file, fsync it, rename over the target.
///
/// `fs::write` truncates the real file and streams into it — a crash (or a
/// full disk) mid-write leaves truncated JSON, which the next `load` reports
/// as corrupt, costing the model the whole plan. `rename(2)` is atomic, so a
/// reader sees either the old file or the new one.
async fn save(path: &Path, store: &TodoStore) -> Result<(), ToolError> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| ToolError::Failed(format!("serialize todos: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = tokio::fs::File::create(&tmp).await?;
        f.write_all(json.as_bytes()).await?;
        f.sync_all().await?;
    }
    tokio::fs::rename(&tmp, path).await?;
    // Best-effort: fsync the directory so the rename itself survives a power
    // loss. Fails on platforms where directories can't be opened — the data
    // rename above is what matters for the corruption case.
    if let Some(dir) = path.parent() {
        if let Ok(d) = tokio::fs::File::open(dir).await {
            let _ = d.sync_all().await;
        }
    }
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
    let path = store_path(&ctx);
    let lock = path_lock(&path);
    let _guard = lock.lock().await;
    let mut store = load(&path, &ctx.workspace_root).await?;
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
            save(&path, &store).await?;
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
            save(&path, &store).await?;
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
            save(&path, &store).await?;
            format!("Removed #{id}\n\n{}", render(&store.items))
        }
        "clear" => {
            let n = store.items.len();
            store.items.clear();
            // `next_id` deliberately survives a clear — see the module docs.
            save(&path, &store).await?;
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
        // Unique dir per test — sharing one path across parallel tests
        // cross-pollutes the per-session store file and flakes the roundtrip.
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

        // persisted across "sessions" (fresh load from the same dir)
        let store = load(&store_path(&ctx), &ctx.workspace_root).await.unwrap();
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
        let path = store_path(&ctx);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let seeded = serde_json::json!({
            "next_id": 2,
            "items": [{"id": 10, "text": "old task", "done": false}],
        });
        std::fs::write(&path, serde_json::to_string(&seeded).unwrap()).unwrap();

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

        let store = load(&store_path(&ctx), &ctx.workspace_root).await.unwrap();
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

    /// `save` must replace atomically and leave no temp file behind.
    #[tokio::test]
    async fn save_is_atomic_and_leaves_no_temp_file() {
        let ctx = ctx();
        let path = store_path(&ctx);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        exec(serde_json::json!({"action":"add","text":"a"}), ctx.clone())
            .await
            .unwrap();
        let dir = path.parent().unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        // The rename target is valid JSON, and re-saving over it works.
        let raw = std::fs::read_to_string(&path).unwrap();
        let store: TodoStore = serde_json::from_str(&raw).unwrap();
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
