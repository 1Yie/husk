//! Memory switch semantics: turning 启用记忆 off mid-session must stop every
//! write path, not just the recall block and the distiller.

use std::sync::Arc;

use agent_llm::types::StreamChunk;
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

fn tool_call(name: &str, args: serde_json::Value) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some(name.into()),
            args_delta: args.to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: Some(5),
            completion_tokens: Some(5),
            cached_tokens: None,
        },
    ]
}

/// Spawn a session, optionally switch memory off, then have the model call
/// `remember`. Returns the persona keys that ended up in the store and the
/// tool result text the model received.
async fn remember_with(toggle_off: bool, key: &str) -> (Vec<String>, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "remember",
        serde_json::json!({ "key": key, "value": "v" }),
    )));
    stub.script_text("done");
    let (mut actor, _ch) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
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
        memory_enabled: true,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "t".into(),
    });
    if toggle_off {
        actor
            .handle(UiCommand::SetMemory {
                enabled: false,
                distill: false,
            })
            .await;
    }
    actor
        .handle(UiCommand::Prompt {
            text: "remember this".into(),
        })
        .await;

    let db = std::env::temp_dir()
        .join(format!("husk-test-{}", std::process::id()))
        .join("memory.db");
    let store = agent_context::memory::MemoryStore::open(&db, dir.path()).unwrap();
    let keys = store
        .all_persona()
        .unwrap()
        .into_iter()
        .map(|p| p.key)
        .collect();
    let results = actor
        .history()
        .iter()
        .filter(|m| m.role == agent_llm::types::Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    (keys, results)
}

// Test binaries share one memory.db per process, so every test uses a key
// that no other test writes and asserts on that key alone.

/// Control: with memory on, `remember` persists the key.
#[tokio::test]
async fn remember_persists_while_memory_is_on() {
    let (keys, _) = remember_with(false, "toggle_on_key").await;
    assert!(keys.contains(&"toggle_on_key".to_string()), "{keys:?}");
}

/// Regression for the toggle bug: after 启用记忆 is switched off, `remember`
/// must refuse and write nothing.
#[tokio::test]
async fn remember_is_refused_after_memory_is_switched_off() {
    let (keys, results) = remember_with(true, "toggle_off_key").await;
    assert!(
        !keys.contains(&"toggle_off_key".to_string()),
        "a write landed with memory off: {keys:?}"
    );
    assert!(
        results.iter().any(|r| r.contains("unavailable")),
        "model should be told memory is unavailable: {results:?}"
    );
}

/// `remember` writes straight into the prompt block, so a credential handed to
/// it must be refused, not stored. The same check the distiller applies.
#[tokio::test]
async fn remember_refuses_a_credential_value() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "remember",
        serde_json::json!({ "key": "leak_probe_key", "value": "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789" }),
    )));
    stub.script_text("done");
    let (mut actor, _ch) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
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
        memory_enabled: true,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "t".into(),
    });
    actor
        .handle(UiCommand::Prompt {
            text: "remember my key".into(),
        })
        .await;

    let db = std::env::temp_dir()
        .join(format!("husk-test-{}", std::process::id()))
        .join("memory.db");
    let store = agent_context::memory::MemoryStore::open(&db, dir.path()).unwrap();
    let keys: Vec<String> = store
        .all_persona()
        .unwrap()
        .into_iter()
        .map(|p| p.key)
        .collect();
    assert!(
        !keys.contains(&"leak_probe_key".to_string()),
        "a credential was persisted: {keys:?}"
    );
}
