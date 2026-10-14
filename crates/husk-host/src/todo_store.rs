//! The host's [`TodoStorage`]: a session's todo list lives beside its history
//! in the session database (`state/<ws>/<id>/todos.json`), so a new session
//! starts clean, a resumed one keeps its list, and the user's repo stays free
//! of agent scratch files.
//!
//! It also adopts a workspace's legacy `{workspace}/.agent/todos.json` the first
//! time a session reads an empty list, and removes that file only once the new
//! row is durable (see [`TodoStorage::seed`]).

use std::path::PathBuf;
use std::sync::Arc;

use agent_store::SessionStore;
use agent_kernel::tools::todo::{TodoStorage, TodoStorageSlot};

const ROW: &str = "todos.json";

pub struct SessionTodoStorage {
    store: Arc<SessionStore>,
    session_id: i64,
    /// The workspace's pre-session-database list, if any.
    legacy: PathBuf,
}

impl SessionTodoStorage {
    pub fn new(store: Arc<SessionStore>, session_id: i64, workspace_root: &std::path::Path) -> Self {
        Self {
            store,
            session_id,
            legacy: workspace_root.join(".agent").join("todos.json"),
        }
    }

    /// Install on a tool context as this session's todo storage.
    pub fn install(self, ctx: &mut agent_kernel::tools::registry::ToolCtx) {
        ctx.ext.insert(Arc::new(TodoStorageSlot(Arc::new(self))));
    }
}

impl TodoStorage for SessionTodoStorage {
    fn read(&self) -> std::io::Result<Option<Vec<u8>>> {
        self.store.read_state(self.session_id, ROW)
    }

    /// One redb commit IS the atomic replace — a crash shows either the
    /// previous row or the new one, never a truncated file.
    fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        self.store.write_state(self.session_id, ROW, bytes)
    }

    fn key(&self) -> String {
        format!(
            "{}:{}:todos",
            self.store.dir_name().unwrap_or_default(),
            self.session_id
        )
    }

    fn seed(&self) -> std::io::Result<Option<Vec<u8>>> {
        match std::fs::read(&self.legacy) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn seeded(&self) {
        let _ = std::fs::remove_file(&self.legacy);
    }
}

#[cfg(test)]
mod tests {
    use agent_capabilities::WorkspaceCtx;
    use super::*;
    use agent_kernel::tools::registry::ToolCtx;

    fn storage(dir: &std::path::Path) -> SessionTodoStorage {
        let store = SessionStore::open_at(dir, &dir.join("t.db")).unwrap();
        SessionTodoStorage::new(Arc::new(store), 0, dir)
    }

    async fn run(ctx: &Arc<ToolCtx>, args: serde_json::Value) -> Result<String, String> {
        let spec = agent_kernel::tools::todo::spec();
        (spec.exec)(args, ctx.clone())
            .await
            .map(|r| r.content)
            .map_err(|e| e.to_string())
    }

    fn ctx_over(storage: SessionTodoStorage, dir: &std::path::Path) -> Arc<ToolCtx> {
        let mut ctx = ToolCtx::new().with_workspace(dir);
        storage.install(&mut ctx);
        Arc::new(ctx)
    }

    /// The list survives in the session database, and a fresh context over the
    /// same store sees it — ids included.
    #[tokio::test]
    async fn the_list_persists_in_the_session_database() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_over(storage(dir.path()), dir.path());
        run(&ctx, serde_json::json!({"action":"add","text":"a"})).await.unwrap();
        run(&ctx, serde_json::json!({"action":"add","text":"b"})).await.unwrap();

        let again = ctx_over(storage(dir.path()), dir.path());
        let out = run(&again, serde_json::json!({"action":"add","text":"c"})).await.unwrap();
        assert!(out.contains("#3"), "ids collided after reload: {out}");
    }

    /// A workspace's legacy `.agent/todos.json` becomes the session's list, and
    /// the file is removed only after the new row exists.
    #[tokio::test]
    async fn a_legacy_workspace_file_is_adopted_then_removed() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(".agent").join("todos.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy,
            r#"{"next_id": 4, "items": [{"id": 3, "text": "old task", "done": true}]}"#,
        )
        .unwrap();

        let s = storage(dir.path());
        let ctx = ctx_over(s, dir.path());
        let out = run(&ctx, serde_json::json!({"action":"list"})).await.unwrap();
        assert!(out.contains("#3 old task"), "{out}");
        assert!(!legacy.exists(), "migration should clean the workspace copy");
        let out = run(&ctx, serde_json::json!({"action":"add","text":"next"})).await.unwrap();
        assert!(out.contains("#4 next"), "{out}");
    }

    /// Unreadable legacy state is NOT an empty list: it surfaces, and the file
    /// stays — it is the only copy.
    #[tokio::test]
    async fn a_corrupt_legacy_file_fails_loudly_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(".agent").join("todos.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{ this is not json").unwrap();

        let ctx = ctx_over(storage(dir.path()), dir.path());
        let err = run(&ctx, serde_json::json!({"action":"list"})).await.unwrap_err();
        assert!(err.contains("legacy"), "{err}");
        assert!(legacy.exists(), "the only copy of the list was deleted");
    }
}
