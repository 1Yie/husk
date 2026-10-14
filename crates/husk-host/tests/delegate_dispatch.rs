//! Regression probe: a model's `delegate` call must dispatch to the builtin
//! registry in Build mode, not fall through to the plugin router.

use std::sync::Arc;

use husk_host::session::{SessionActor, SessionConfig};
use agent_ipc::UiCommand;
use agent_llm::types::StreamChunk;
mod common;
use common::ScriptedProvider;

fn test_paths() -> agent_paths::AppPaths {
    let base = std::env::temp_dir().join("husk-plugin-test");
    agent_paths::AppPaths::new(base.join("config"), base.join("data"), ".husk")
}

#[tokio::test]
async fn delegate_call_in_build_mode_is_a_builtin_not_a_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    // Mirror the screenshot: an earlier tool call in the same turn, then delegate.
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_0".into()),
            name: Some("list_dir".into()),
            args_delta: r#"{"path":"."}"#.into(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]);
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("delegate".into()),
            args_delta: r#"{"agent":"review","task":"逐个阅读 crates/agent-kernel/src/tools/ 下的所有 .rs 文件"}"#.into(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]);
    provider.script_text("done");

    let (mut actor, mut channels) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider: provider.clone(),
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "auto".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    });
    actor.handle(UiCommand::Prompt { text: "go".into() }).await;

    let mut finished = Vec::new();
    while let Some(ev) = channels.event_rx.try_recv() {
        if let agent_ipc::UiEvent::ToolCallFinished { name, ok, content, .. } = ev {
            finished.push((name, ok, content));
        }
    }
    // Without this the probe could pass while `delegate` never ran at all.
    let delegate = finished
        .iter()
        .find(|(n, _, _)| n == "delegate")
        .unwrap_or_else(|| panic!("delegate never finished: {finished:?}"));
    assert!(
        !delegate.2.contains("no plugin owns tool"),
        "delegate fell through to the plugin router: {delegate:?}"
    );
    assert!(delegate.1, "delegate ran but failed: {delegate:?}");
}

/// When a built-in call fails and no plugin owns the name, the user must see the
/// built-in's real error. `delegate` with no `task` is a built-in argument error,
/// and the plugin router (empty here) must not replace it with "no plugin owns".
#[tokio::test]
async fn builtin_failure_is_not_masked_by_plugin_no_owner_message() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("delegate".into()),
            args_delta: r#"{}"#.into(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]);
    provider.script_text("done");

    let handle: agent_kernel::engine::PluginHandle = Arc::new(std::sync::RwLock::new(Some(
        Arc::new(agent_plugin::PluginManager::new(test_paths())),
    )));
    let (mut actor, _ch) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider: provider.clone(),
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "bypass".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: Some(handle),
        provider_name: "test".into(),
    });
    actor.handle(UiCommand::Prompt { text: "go".into() }).await;

    let results: Vec<String> = actor
        .history()
        .iter()
        .filter(|m| m.role == agent_llm::types::Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    assert!(!results.is_empty(), "delegate produced no tool result");
    assert!(
        !results.iter().any(|r| r.contains("no plugin owns tool")),
        "built-in failure was masked by the plugin message: {results:?}"
    );
}
