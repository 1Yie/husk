//! Storage tombstones stop deleted actors from recreating history or draining
//! queued tasks. The real actor path also verifies durable /clear behavior.

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_kernel::session_store::SessionStore;
use agent_llm::types::StreamChunk;
use std::sync::Arc;
use std::time::Duration;
mod common;
use common::ScriptedProvider;

fn cfg(root: &std::path::Path, provider: Arc<ScriptedProvider>) -> SessionConfig {
    SessionConfig {
        workspace_root: root.into(),
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
        compact_at: None,
        model_input: vec![],
        model_params: None,
        plugins: None,
        queued_prompts: vec![],
        memory_enabled: false,
        memory_distill: false,
    }
}

#[tokio::test]
async fn deleted_running_actor_exits_without_persistence_or_queued_turns() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![StreamChunk::ToolCallDelta {slot:0,id:Some("write".into()),name:Some("apply_patch".into()),args_delta:serde_json::json!({"patch":"*** Begin Patch\n*** Add File: out.txt\n+no\n*** End Patch\n"}).to_string()},StreamChunk::Done {prompt_tokens:None,completion_tokens:None,cached_tokens:None}]);
    let (mut actor, mut channels) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    let tx = actor.command_sender();
    let cancel = actor.cancel_writer();
    let shutdown = actor.shutdown_writer();
    let queue = actor.queue_writer();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt {
        text: "first".into(),
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = channels.event_rx.recv().await {
            if matches!(event, UiEvent::ApprovalRequested { .. }) {
                return;
            }
        }
        panic!("approval not reached");
    })
    .await
    .unwrap();
    queue.lock().unwrap().push_back("must not run".into());
    store.remove(id).unwrap();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    shutdown.notify_one();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(store.load_history(id).is_none());
    assert!(!store.list().iter().any(|m| m.id == id));
    assert!(!dir.path().join("out.txt").exists());
    while let Some(event) = channels.event_rx.try_recv() {
        assert!(!matches!(event,UiEvent::UserPrompt(ref t) if t=="must not run"));
    }
    assert!(channels.event_rx.recv().await.is_none());
}

#[tokio::test]
async fn clear_history_is_committed_before_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("old answer");
    let (mut actor, _) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    actor
        .handle(UiCommand::Prompt {
            text: "old prompt".into(),
        })
        .await;
    assert!(store
        .load_history(id)
        .unwrap()
        .iter()
        .any(|m| m.content.as_deref() == Some("old prompt")));
    actor
        .handle(UiCommand::Prompt {
            text: "/clear".into(),
        })
        .await;
    assert!(!store
        .load_history(id)
        .unwrap()
        .iter()
        .any(|m| m.content.as_deref() == Some("old prompt")));
    assert_eq!(store.load_history(id).unwrap().len(), actor.history().len());
}

fn write_script(provider: &ScriptedProvider, patch: &str) {
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("w".into()),
            name: Some("apply_patch".into()),
            args_delta: serde_json::json!({ "patch": patch }).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]);
    provider.script_text("done");
}

/// Drive one apply_patch turn up to the approval card, run `between`, then
/// approve. Returns once the turn is idle again.
async fn approve_after<F: FnOnce()>(root: &std::path::Path, patch: &str, between: F) {
    let store = SessionStore::open(root).unwrap();
    let id = store.next_id();
    let provider = Arc::new(ScriptedProvider::new());
    write_script(&provider, patch);
    let (mut actor, mut channels) = SessionActor::spawn_with_id(cfg(root, provider), id);
    let tx = actor.command_sender();
    let decision = actor.decision_writer();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt { text: "go".into() }).await.unwrap();
    let request_id = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = channels.event_rx.recv().await {
            if let UiEvent::ApprovalRequested { request_id, .. } = event {
                return request_id;
            }
        }
        panic!("approval not reached");
    })
    .await
    .unwrap();
    between();
    *decision.lock().unwrap() = Some((request_id, true));
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = channels.event_rx.recv().await {
            if matches!(event, UiEvent::StateChanged(agent_ipc::AgentState::Finished)) {
                return;
            }
        }
    })
    .await
    .ok();
    drop(tx);
    task.abort();
}

/// K1: a file edited while the approval card is open must not be overwritten
/// with content computed from the stale pre-image.
#[tokio::test]
async fn approval_does_not_overwrite_a_file_edited_after_the_preview() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "one\n").unwrap();
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+two\n*** End Patch\n";
    approve_after(dir.path(), patch, || {
        std::fs::write(dir.path().join("a.txt"), "user edit\n").unwrap();
    })
    .await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "user edit\n");
}

/// Control: with no concurrent edit the approved write still lands.
#[tokio::test]
async fn approval_still_applies_when_file_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "one\n").unwrap();
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+two\n*** End Patch\n";
    approve_after(dir.path(), patch, || {}).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "two\n");
}
