//! `SessionManager::active_children` — what a webview that reloaded mid-turn
//! gets back: the subagents its session is still running, with the label, task
//! and question their cards need. The live events that drew the cards are gone
//! and history holds no row for a child that has not reported yet.

use std::sync::Arc;
use std::time::Duration;

use agent_capabilities::{WorkspaceCtx, WorkspacePaths};
use agent_kernel::tools::delegate::{ChildRoute, SubagentSpawner};
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use agent_testkit::ScriptedProvider;
use husk_host::session_manager::SessionManager;

fn spawner(provider: Arc<dyn agent_llm::LlmProvider>) -> SubagentSpawner {
    let full = husk_host::session::base_registries().1;
    let readonly = full.readonly_only();
    let route = Arc::new(std::sync::RwLock::new(ChildRoute {
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        context_window: 256_000,
        thinking_level: None,
        thinking_level_map: None,
        model_params: agent_llm::ModelParams::default(),
        model_input: Vec::new(),
        models: std::collections::HashMap::new(),
    }));
    SubagentSpawner::new(route, Arc::new(full), Arc::new(readonly))
        .with_host(Arc::new(husk_host::subagents::HuskSubagents))
}

fn registry() -> ToolRegistry {
    husk_host::session::base_registries().0
}

#[tokio::test]
async fn a_running_child_is_listed_for_the_session_that_owns_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = agent_paths::AppPaths::new(
        dir.path().join("config"),
        dir.path().join("data"),
        ".husk",
    );
    let (manager, _events) = SessionManager::spawn_at(Some(dir.path().to_path_buf()), paths);
    let root = manager.workspace_root.to_string_lossy().into_owned();
    let id = manager.active_id;
    assert!(id > 0, "boot leaves one live session");
    assert!(
        manager.active_children(&root, id).is_empty(),
        "nothing was delegated yet"
    );
    // No live handle → no children. A child cannot outlive the process, and a
    // parked workspace's same-numbered session is not this manager's.
    assert!(manager.active_children(&root, id + 1_000).is_empty());

    // A child held on its first request: registered, genuinely mid-turn.
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("done");
    let gate = Arc::new(tokio::sync::Notify::new());
    *provider.gate.lock().unwrap() = Some(gate.clone());
    let mut ctx = ToolCtx::new().with_workspace(dir.path());
    ctx.install_paths(agent_paths::AppPaths::new(dir.path(), dir.path(), ".husk"));
    ctx.install_subagent(spawner(provider.clone()));
    let ctx = Arc::new(ctx.with_session(id));
    let run = tokio::spawn(async move {
        let spec = registry().spec("delegate").unwrap();
        (spec.exec)(serde_json::json!({"task": "看鉴权分支"}), ctx).await
    });

    let listed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let children = manager.active_children(&root, id);
            if !children.is_empty() {
                return children;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the running child never showed up");
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].label, "subagent");
    assert_eq!(listed[0].task, "看鉴权分支");
    assert_eq!(listed[0].waiting, None, "not parked");

    gate.notify_one();
    run.await.unwrap().expect("the child must finish");
    assert!(
        manager.active_children(&root, id).is_empty(),
        "a finished child is not running"
    );
}
