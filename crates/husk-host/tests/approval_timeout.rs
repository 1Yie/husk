//! A card nobody answers must not strand the turn — the session's configured
//! bounds reach the engine, which denies the call by itself and lets the model
//! carry on.

use std::sync::Arc;
use std::time::Duration;

use agent_ipc::{AgentState, UiEvent};
use agent_llm::types::StreamChunk;
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

#[tokio::test]
async fn an_unanswered_approval_times_out_and_ends_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("write".into()),
            name: Some("apply_patch".into()),
            args_delta: serde_json::json!({
                "patch": "*** Begin Patch\n*** Add File: out.txt\n+test\n*** End Patch\n"
            })
            .to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]);
    provider.script_text("done");

    // `default` mode asks before the write; nothing ever answers the card.
    let (mut actor, mut channels) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider,
        provider_name: "test".into(),
        model: "test".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        model_input: Vec::new(),
        compact_at: None,
        approval_timeout: Some(Duration::from_secs(1)),
        question_timeout: Some(Duration::from_secs(60)),
        model_params: None,
        plugins: None,
        queued_prompts: Vec::new(),
        memory_enabled: false,
        memory_distill: false,
    });

    tokio::time::timeout(
        Duration::from_secs(20),
        actor.handle(UiCommand::Prompt {
            text: "write it".into(),
        }),
    )
    .await
    .expect("the configured bound must end the turn instead of hanging it");

    channels.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        events.push(ev);
    }

    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::ApprovalTimedOut { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::ToolCallFinished { ok: false, content, .. } if content.contains("审批超时")
    )));
    assert!(!dir.path().join("out.txt").exists());
    assert!(matches!(
        events
            .iter()
            .rev()
            .find_map(|e| match e {
                UiEvent::StateChanged(s) => Some(s),
                _ => None,
            }),
        Some(AgentState::Finished)
    ));
}
