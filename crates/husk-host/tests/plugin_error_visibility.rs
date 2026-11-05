//! Plugin tool status and diagnostic text reach the model, UI and saved history.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, RwLock};

use agent_ipc::UiEvent;
use agent_llm::types::{Role, StreamChunk};
use agent_store::SessionStore;
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};
use serde_json::{json, Value};

fn tool_call(name: &str) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some(name.into()),
            args_delta: "{}".into(),
        },
        StreamChunk::Done {
            prompt_tokens: Some(5),
            completion_tokens: Some(5),
            cached_tokens: None,
        },
    ]
}

/// A stdio MCP server that returns the configured JSON-RPC response body.
fn failing_server(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("srv.sh");
    std::fs::write(
        &p,
        r#"#!/bin/sh
while read -r line; do
  id=$(echo "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *initialize*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":\"p\",\"version\":\"1\"}}}" ;;
    *tools/list*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"tools\":[{\"name\":\"boom\",\"description\":\"d\",\"inputSchema\":{\"type\":\"object\",\"properties\":{}}}]}}" ;;
    *tools/call*)
      printf 'call\n' >> "$CALL_LOG"
      printf '{"jsonrpc":"2.0","id":%s,%s}\n' "$id" "$TOOL_REPLY" ;;
  esac
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[tokio::test(flavor = "multi_thread")]
async fn model_sees_the_plugins_own_error_not_unknown_tool() {
    assert_plugin_result(
        json!({"error": {"code": -32000, "message": "DATABASE_QUOTA_EXCEEDED: retry after 60s"}}),
        "DATABASE_QUOTA_EXCEEDED: retry after 60s",
        true,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_application_errors_reach_events_and_persisted_history() {
    assert_plugin_result(
        json!({"result": {"isError": true, "content": [
            {"type": "text", "text": "DATABASE_QUOTA_EXCEEDED"},
            {"type": "text", "text": "retry after 60s"}
        ]}}),
        "DATABASE_QUOTA_EXCEEDED\nretry after 60s\n",
        true,
    )
    .await;
    assert_plugin_result(
        json!({"result": {"isError": true, "content": []}}),
        "MCP tool `boom` failed without error content",
        true,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_success_keeps_success_events_and_history() {
    for result in [
        json!({"isError": false, "content": [{"type": "text", "text": "success"}]}),
        json!({"content": [{"type": "text", "text": "success"}]}),
    ] {
        assert_plugin_result(json!({"result": result}), "success\n", false).await;
    }
}

async fn assert_plugin_result(reply: Value, expected: &str, is_error: bool) {
    let plugin_dir = tempfile::tempdir().unwrap();
    let srv = failing_server(plugin_dir.path());
    let reply = serde_json::to_string(&reply).unwrap();
    let mut m: agent_plugin::PluginManifest = serde_json::from_value(serde_json::json!({
        "id": "flaky", "name": "flaky", "kind": "mcp",
        "entry": {
            "command": srv.to_string_lossy(), "args": [],
            "env": {
                "TOOL_REPLY": &reply[1..reply.len() - 1],
                "CALL_LOG": plugin_dir.path().join("calls")
            }
        },
    }))
    .unwrap();
    m.dir = plugin_dir.path().to_path_buf();
    let paths = agent_paths::AppPaths::new(
        plugin_dir.path().join("config"),
        plugin_dir.path().join("data"),
        ".husk",
    );
    let mut mgr = agent_plugin::PluginManager::new(paths.clone());
    mgr.register_plugin(m, plugin_dir.path()).await.unwrap();
    let runtime: husk_host::plugins::RuntimePluginHandle = Arc::new(RwLock::new(None));
    let handle = husk_host::plugins::router_handle(runtime.clone());
    assert!(handle
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .exported_tools()
        .is_empty());
    // An already-installed adapter must see the runtime that a reload installs.
    *runtime.write().unwrap() = Some(Arc::new(mgr));
    assert!(!handle
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .exported_tools()
        .is_empty());

    let ws = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call("flaky__boom")));
    stub.script_text("done");
    let config = || SessionConfig {
        workspace_root: ws.path().to_path_buf(),
        paths: paths.clone(),
        provider: stub.clone(),
        model: "m".into(),
        temperature: 0.0,
        permission_mode: "bypass".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        approval_timeout: None,
        question_timeout: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: Some(runtime.clone()),
        provider_name: "t".into(),
    };
    let (mut actor, mut channels) = SessionActor::spawn_with_id(config(), 7);
    actor
        .handle(UiCommand::Prompt {
            text: "call the flaky tool".into(),
        })
        .await;

    let results: Vec<String> = actor
        .history()
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    assert_eq!(results.len(), 1, "{results:?}");
    let shown = &results[0];
    assert!(
        shown.contains(expected),
        "the plugin's own error must reach the model: {shown:?}"
    );
    assert!(
        !shown.contains("unknown tool"),
        "a registered plugin tool must not read as unknown: {shown:?}"
    );
    let tool = actor
        .history()
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert_eq!(tool.is_error.unwrap_or(false), is_error);
    channels.event_rx.close();
    let mut tool_events = Vec::new();
    while let Some(event) = channels.event_rx.recv().await {
        if let UiEvent::ToolCallFinished { ok, content, .. } = event {
            tool_events.push((ok, content));
        }
    }
    assert_eq!(tool_events.len(), 1, "{tool_events:?}");
    assert_eq!(tool_events[0].0, !is_error);
    assert!(tool_events[0].1.contains(expected), "{tool_events:?}");

    let store = SessionStore::open(ws.path()).unwrap();
    let persisted = store.load_history(7).expect("session snapshot");
    let saved_tool = persisted.iter().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(saved_tool.content, tool.content);
    assert_eq!(saved_tool.is_error, tool.is_error);
    let (restored, _) = SessionActor::resume(config(), 7, persisted);
    let restored_tool = restored
        .history()
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert_eq!(restored_tool.content, tool.content);
    assert_eq!(restored_tool.is_error, tool.is_error);
    assert_eq!(
        std::fs::read_to_string(plugin_dir.path().join("calls")).unwrap(),
        "call\n",
        "a tool result must not trigger an automatic retry"
    );
}
