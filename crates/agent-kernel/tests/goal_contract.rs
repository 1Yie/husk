//! Goal contract: in `goal` mode a turn that goes quiet without declaring a
//! completion or a blocker is pushed back to work, but only a bounded number
//! of times. These tests pin that cap so the turn loop can be restructured
//! without changing how often the model is nudged.
use std::sync::Arc;

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_llm::types::NoticeKind;

mod common;
use common::ScriptedProvider;

fn spawn(
    dir: &std::path::Path,
    stub: Arc<ScriptedProvider>,
    agent_mode: &str,
) -> (SessionActor, agent_kernel::channels::UiChannels) {
    SessionActor::spawn(SessionConfig {
        workspace_root: dir.to_path_buf(),
        provider: stub,
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "bypassPermissions".into(),
        agent_mode: agent_mode.into(),
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
    })
}

fn hidden_texts(actor: &SessionActor) -> Vec<String> {
    actor
        .history()
        .iter()
        .filter(|m| m.notice == Some(NoticeKind::Hidden))
        .filter_map(|m| m.content.clone())
        .filter(|c| c.contains("目标尚未宣告完成"))
        .collect()
}

/// A model that never declares and never blocks is nudged exactly
/// `MAX_GOAL_FOLLOWUPS` times, then the turn is allowed to end.
#[tokio::test]
async fn undeclared_goal_is_nudged_up_to_the_cap_then_released() {
    let dir = tempfile::tempdir().unwrap();
    // Every sampling round answers in prose and calls no tool.
    let stub = Arc::new(ScriptedProvider::new());
    for _ in 0..20 {
        stub.script_text("still thinking");
    }
    let (mut actor, mut ch) = spawn(dir.path(), stub.clone(), "goal");

    actor
        .handle(UiCommand::Prompt {
            text: "make progress".into(),
        })
        .await;

    // One initial round plus one per nudge.
    const CAP: usize = 8;
    assert_eq!(hidden_texts(&actor).len(), CAP, "nudge count");
    assert_eq!(stub.calls.lock().unwrap().len(), CAP + 1, "sampling rounds");

    // The turn did finish: the terminal state was reached.
    ch.event_rx.close();
    let mut finished = false;
    while let Some(ev) = ch.event_rx.recv().await {
        if matches!(ev, UiEvent::AssistantMessage(_)) {
            finished = true;
        }
    }
    assert!(finished, "the released turn must end with an answer");
}
