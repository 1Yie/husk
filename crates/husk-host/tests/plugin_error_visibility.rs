//! A plugin tool that fails must tell the model WHY. The engine used to drop
//! the plugin's own error and report the built-in registry's "unknown tool"
//! instead, so a quota error read as a non-existent tool and the model had
//! nothing to act on.

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, RwLock};

use agent_llm::types::{Role, StreamChunk};
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

fn test_paths() -> agent_paths::AppPaths {
    let base = std::env::temp_dir().join("husk-plugin-test");
    agent_paths::AppPaths::new(base.join("config"), base.join("data"), ".husk")
}

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

/// A stdio MCP server whose one tool always answers with a JSON-RPC error.
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
    *tools/call*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32000,\"message\":\"DATABASE_QUOTA_EXCEEDED: retry after 60s\"}}" ;;
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
    let plugin_dir = tempfile::tempdir().unwrap();
    let srv = failing_server(plugin_dir.path());
    let mut m: agent_plugin::PluginManifest = serde_json::from_value(serde_json::json!({
        "id": "flaky", "name": "flaky", "kind": "mcp",
        "entry": { "command": srv.to_string_lossy(), "args": [] },
    }))
    .unwrap();
    m.dir = plugin_dir.path().to_path_buf();
    let mut mgr = agent_plugin::PluginManager::new(test_paths());
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
    let (mut actor, _ch) = SessionActor::spawn(SessionConfig {
        workspace_root: ws.path().to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
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
        plugins: Some(runtime),
        provider_name: "t".into(),
    });
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
        shown.contains("DATABASE_QUOTA_EXCEEDED"),
        "the plugin's own error must reach the model: {shown:?}"
    );
    assert!(
        !shown.contains("unknown tool"),
        "a registered plugin tool must not read as unknown: {shown:?}"
    );
}
