//! Session-level recall semantics: the per-message `<memory-context>` row is a
//! delta. A fact the model has already read is not sent again, and a
//! `remember` written mid-session reaches the model on the next message
//! instead of waiting for a restart.

use std::sync::Arc;

use agent_llm::types::StreamChunk;
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

/// Every test in this binary shares one `memory.db` (one per process), so each
/// writes facts and keys under a token no other test uses.
fn store_for(workspace: &std::path::Path) -> agent_context::memory::MemoryStore {
    let dir = std::env::temp_dir().join(format!("husk-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    agent_context::memory::MemoryStore::open(&dir.join("memory.db"), workspace).unwrap()
}

fn session(workspace: &std::path::Path, stub: Arc<ScriptedProvider>) -> SessionActor {
    let (actor, _ch) = SessionActor::spawn(SessionConfig {
        workspace_root: workspace.to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider: stub,
        model: "m".into(),
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
        memory_enabled: true,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "t".into(),
    });
    actor
}

/// The hidden recall rows this session has sent.
fn recall_rows(actor: &SessionActor) -> Vec<String> {
    actor
        .history()
        .iter()
        .filter(|m| m.role == agent_llm::types::Role::User)
        .filter_map(|m| m.content.clone())
        .filter(|c| c.starts_with("<memory-context>"))
        .collect()
}

fn remember_call(key: &str, value: &str) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some("remember".into()),
            args_delta: serde_json::json!({ "key": key, "value": value }).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: Some(5),
            completion_tokens: Some(5),
            cached_tokens: None,
        },
    ]
}

/// One fact, two prompts that match it: the model reads it once.
#[tokio::test]
async fn a_recalled_fact_is_not_sent_twice() {
    let dir = tempfile::tempdir().unwrap();
    store_for(dir.path())
        .upsert_fact("zzpingu alpha beta convention", 0.6)
        .unwrap();

    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("ok");
    stub.script_text("ok");
    let mut actor = session(dir.path(), stub);

    actor
        .handle(UiCommand::Prompt {
            text: "zzpingu alpha".into(),
        })
        .await;
    let first = recall_rows(&actor);
    assert!(
        first.iter().any(|r| r.contains("zzpingu")),
        "the matching fact was never injected: {first:?}"
    );

    actor
        .handle(UiCommand::Prompt {
            text: "zzpingu alpha".into(),
        })
        .await;
    let sent = recall_rows(&actor)
        .iter()
        .filter(|r| r.contains("zzpingu"))
        .count();
    assert_eq!(sent, 1, "the same fact was re-sent: {:?}", recall_rows(&actor));
}

/// The stable block is rendered once per spawn, so a `remember` mid-session
/// has to reach the model through the recall row — otherwise the tool's own
/// contract ("it is remembered") is false for the rest of the conversation.
#[tokio::test]
async fn a_mid_session_remember_reaches_the_next_message() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(
        ScriptedProvider::new().with_script(remember_call("nxmsg_key", "nxmsg value")),
    );
    stub.script_text("done");
    stub.script_text("ok");
    let mut actor = session(dir.path(), stub);

    actor
        .handle(UiCommand::Prompt {
            text: "记住 nxmsg value".into(),
        })
        .await;

    actor
        .handle(UiCommand::Prompt {
            text: "现在呢".into(),
        })
        .await;
    let rows = recall_rows(&actor);
    assert!(
        rows.iter().any(|r| r.contains("nxmsg_key: nxmsg value")),
        "the fresh persona never reached the model: {rows:?}"
    );
}
