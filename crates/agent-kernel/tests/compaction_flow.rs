//! Compaction acceptance: `/compact` must actually rewrite the context.
//!
//! Regression guard for the "button does nothing" shape: the command used
//! to only *report* a threshold (and the trigger estimate was never
//! re-based), so the history — and therefore the persisted snapshot the
//! raw-JSON viewer reads — did not change. This drives the real actor on a
//! scripted provider and asserts the rewrite, the card event, and the
//! persisted snapshot.

use std::sync::Arc;

use agent_ipc::{AgentState, UiCommand, UiEvent};
use agent_kernel::compaction::estimate_tokens;
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_kernel::session_store::SessionStore;
use agent_llm::types::{ChatMessage, NoticeKind, StreamChunk};

#[allow(dead_code)] // Shared harness — this binary uses only part of it.
mod common;
use common::ScriptedProvider;

const SUMMARY: &str = "MERGED-NOTE: earlier turns covered alpha and beta decisions.";
const SESSION_ID: i64 = 7;

/// History with one big prefix turn: ≈2000 tokens in `a1`, then three tiny
/// turns. Against a 4k window the ~25% suffix keeps the tiny tail and folds
/// `q1`/`a1` — and the 80% trigger stays clear, so only `/compact` can fire.
fn seeded_history() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("kernel"),
        ChatMessage::user("q1"),
        ChatMessage::assistant("A".repeat(8_000)),
        ChatMessage::user("q2"),
        ChatMessage::assistant("a2"),
        ChatMessage::user("q3"),
        ChatMessage::assistant("a3"),
        ChatMessage::user("q4"),
        ChatMessage::assistant("a4"),
    ]
}

fn spawn(
    cfg_root: &std::path::Path,
    stub: Arc<ScriptedProvider>,
) -> (SessionActor, agent_kernel::channels::UiChannels) {
    SessionActor::resume(
        SessionConfig {
            workspace_root: cfg_root.to_path_buf(),
            provider: stub,
            model: "test-model".into(),
            temperature: 0.0,
            permission_mode: "default".into(),
            agent_mode: "build".into(),
            track_dirty: false,
            thinking_level: None,
            thinking_level_map: None,
            context_window: Some(4_000),
            compact_at: None,
            model_input: Vec::new(),
            model_params: None,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        },
        SESSION_ID,
        seeded_history(),
    )
}

#[tokio::test]
async fn manual_compact_rewrites_the_context_and_persists_it() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());
    // The only sample this test allows: the compactor's own call.
    stub.script_text(SUMMARY);

    let (mut actor, mut channels) = spawn(dir.path(), stub);

    let before_tokens = estimate_tokens(actor.history());
    assert!(
        before_tokens > 2_000,
        "fixture must carry real history: {before_tokens}"
    );

    actor
        .handle(UiCommand::Prompt {
            text: "/compact".into(),
        })
        .await;

    // (1) The context actually shrank.
    let after_tokens = estimate_tokens(actor.history());
    assert!(
        after_tokens < before_tokens / 2,
        "compaction must rewrite the context: {before_tokens} → {after_tokens}"
    );

    // (2) The model continues from the summary note at index 1 — the kernel
    //     system prompt at index 0 survives.
    assert_eq!(actor.history()[0].content.as_deref(), Some("kernel"));
    assert_eq!(actor.history()[1].notice, Some(NoticeKind::CompactedMemory));
    assert!(actor.history()[1]
        .content
        .as_deref()
        .unwrap()
        .contains(SUMMARY));

    // (3) The display card is persisted as a trailing row.
    let card = actor.history().last().unwrap();
    assert_eq!(card.notice, Some(NoticeKind::Compacted));
    let payload: serde_json::Value =
        serde_json::from_str(card.content.as_deref().unwrap()).unwrap();
    assert_eq!(payload["note"], SUMMARY);
    assert_eq!(payload["removed_messages"], 2);
    assert!(payload["before_tokens"].as_u64().unwrap() > payload["after_tokens"].as_u64().unwrap());

    // (4) The snapshot on disk matches — the raw-JSON viewer and a reopened
    //     session read THIS, not the in-memory copy.
    let store = SessionStore::open(dir.path()).unwrap();
    let persisted = store.load_history(SESSION_ID).expect("snapshot must exist");
    assert_eq!(persisted.len(), actor.history_len());
    assert_eq!(
        persisted.last().unwrap().notice,
        Some(NoticeKind::Compacted),
        "the persisted snapshot must carry the post-compaction history"
    );
    assert!(estimate_tokens(&persisted) < before_tokens / 2);

    // (5) The live card event carries the same accounting.
    channels.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        events.push(ev);
    }
    let card_event = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Compacted {
                before_tokens,
                after_tokens,
                removed_messages,
                manual,
                note,
                ..
            } => Some((
                *before_tokens,
                *after_tokens,
                *removed_messages,
                *manual,
                note.clone(),
            )),
            _ => None,
        })
        .expect("/compact must emit UiEvent::Compacted");
    assert_eq!(card_event.4, SUMMARY);
    assert!(card_event.0 > card_event.1);
    assert_eq!(card_event.2, 2);
    // A user-run pass is tagged manual — the UI renders it as its own block
    // instead of folding it into the previous answer.
    assert!(card_event.3, "a `/compact` pass must be flagged manual");
    assert_eq!(payload["manual"], true);
    // (6) The in-flight phase is published before the card — without it the
    //     UI had nothing to show while the summarization sample ran.
    let compacting_at = events
        .iter()
        .position(|e| matches!(e, UiEvent::StateChanged(AgentState::Compacting)))
        .expect("/compact must announce the Compacting phase");
    let card_at = events
        .iter()
        .position(|e| matches!(e, UiEvent::Compacted { .. }))
        .unwrap();
    assert!(compacting_at < card_at, "the phase must precede the card");
    // ...and the phase is restored after the pass (settled again).
    let restored = events
        .iter()
        .skip(card_at + 1)
        .any(|e| matches!(e, UiEvent::StateChanged(s) if !s.is_active()));
    assert!(
        restored,
        "the pre-command phase must come back after the pass"
    );
    // No stale "compaction failed" line leaked into the stream.
    assert!(events
        .iter()
        .all(|e| !matches!(e, UiEvent::SystemMessage(m) if m.contains("失败"))));
}

#[tokio::test]
async fn compact_on_a_short_history_is_a_neutral_noop() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());

    let (mut actor, mut channels) = SessionActor::resume(
        SessionConfig {
            workspace_root: dir.path().to_path_buf(),
            provider: stub,
            model: "test-model".into(),
            temperature: 0.0,
            permission_mode: "default".into(),
            agent_mode: "build".into(),
            track_dirty: false,
            thinking_level: None,
            thinking_level_map: None,
            context_window: Some(256_000),
            compact_at: None,
            model_input: Vec::new(),
            model_params: None,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        },
        SESSION_ID,
        vec![
            ChatMessage::system("kernel"),
            ChatMessage::user("q1"),
            ChatMessage::assistant("a1"),
        ],
    );

    let before = actor.history_len();
    actor
        .handle(UiCommand::Prompt {
            text: "/compact".into(),
        })
        .await;
    // Only the neutral "nothing to do" notice is appended — nothing was
    // compacted away, and the line must NOT read as a failure.
    assert_eq!(actor.history_len(), before + 1);
    let last = actor.history().last().unwrap();
    assert_eq!(last.notice, Some(NoticeKind::System));
    assert!(last.content.as_deref().unwrap().contains("无需压缩"));
    assert!(actor
        .history()
        .iter()
        .all(|m| m.notice != Some(NoticeKind::Compacted)));

    channels.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        events.push(ev);
    }
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::SystemMessage(m) if m.contains("无需压缩"))));
    assert!(events
        .iter()
        .all(|e| !matches!(e, UiEvent::Compacted { .. })));
}

/// A long agentic turn (tool → result → tool …) grows the context without
/// ever returning to a prompt. The trigger must fire between tool rounds —
/// otherwise the only guard left is `fit_conversation_to_budget` silently
/// dropping the oldest messages at 90%.
#[tokio::test]
async fn auto_compaction_fires_between_tool_rounds_mid_turn() {
    let dir = tempfile::tempdir().unwrap();
    // Reading this file pushes the estimate past the 80% trigger.
    let body: String = (0..200)
        .map(|i| format!("line {i}: {}\n", "x".repeat(9)))
        .collect();
    std::fs::write(dir.path().join("big.txt"), body).unwrap();

    let stub = Arc::new(ScriptedProvider::new());
    let done = |p: u32, c: u32| StreamChunk::Done {
        prompt_tokens: Some(p),
        completion_tokens: Some(c),
        cached_tokens: None,
    };
    // Round 1: the model reads the file.
    let tool_args = serde_json::json!({"path": "big.txt"}).to_string();
    stub.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("smart_read".into()),
            args_delta: tool_args,
        },
        done(3_000, 20),
    ]);
    // The compactor's own sample. If the mid-turn trigger never fired, the
    // next round would consume THIS script as the answer — the assertion on
    // the final text below is what proves the pass ran.
    stub.push_script(vec![
        StreamChunk::ContentDelta(SUMMARY.into()),
        done(1_500, 20),
    ]);
    // Round 2: the final answer.
    stub.push_script(vec![
        StreamChunk::ContentDelta("final answer after compaction".into()),
        done(900, 10),
    ]);

    let (mut actor, mut channels) = SessionActor::resume(
        SessionConfig {
            workspace_root: dir.path().to_path_buf(),
            provider: stub,
            model: "test-model".into(),
            temperature: 0.0,
            permission_mode: "default".into(),
            agent_mode: "build".into(),
            track_dirty: false,
            thinking_level: None,
            thinking_level_map: None,
            // 80% trigger = 3200; the seeded prefix sits just below it, and
            // the tool result is what crosses it.
            context_window: Some(4_000),
            compact_at: None,
            model_input: Vec::new(),
            model_params: None,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        },
        SESSION_ID,
        vec![
            ChatMessage::system("kernel"),
            ChatMessage::user("q1"),
            ChatMessage::assistant("A".repeat(12_000)), // ≈3000 tokens
        ],
    );

    actor
        .handle(UiCommand::Prompt {
            text: "read big.txt".into(),
        })
        .await;

    channels.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        events.push(ev);
    }

    // The turn finished with the round-2 answer — not with the summary the
    // compactor produced.
    let final_text = events
        .iter()
        .rev()
        .find_map(|e| match e {
            UiEvent::AssistantMessage(t) => Some(t.clone()),
            _ => None,
        })
        .expect("turn must produce an assistant message");
    assert_eq!(
        final_text, "final answer after compaction",
        "the compactor script leaked into the answer — the mid-turn pass did not run"
    );
    let auto_manual = events.iter().find_map(|e| match e {
        UiEvent::Compacted { manual, .. } => Some(*manual),
        _ => None,
    });
    assert_eq!(
        auto_manual,
        Some(false),
        "the mid-turn pass must emit the card, flagged automatic"
    );
    // The running-card signal precedes it and carries the same origin.
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::CompactionStarted { manual: false })));
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::StateChanged(AgentState::Compacting))));

    // The rewritten history keeps the note + card row, and the tool result
    // (with its call) survives in the verbatim suffix — never orphaned.
    assert!(actor
        .history()
        .iter()
        .any(|m| m.notice == Some(NoticeKind::CompactedMemory)));
    assert!(actor
        .history()
        .iter()
        .any(|m| m.notice == Some(NoticeKind::Compacted)));
    assert!(actor
        .history()
        .iter()
        .any(|m| m.role == agent_llm::types::Role::Tool));
}

/// The 70/80/90% setting must retune a RUNNING session, not only sessions
/// spawned afterwards: it is a workspace-wide rule with no per-session
/// override, so a live actor has to pick it up from `SetCompactAt`.
#[tokio::test]
async fn compact_ratio_change_applies_to_a_live_session() {
    // ≈2880 tokens: above 70% of a 4k window (2800), below 80% (3200).
    let seeded = || {
        vec![
            ChatMessage::system("kernel"),
            ChatMessage::user("q1"),
            ChatMessage::assistant("A".repeat(11_500)),
        ]
    };
    let config = |root: &std::path::Path, stub: Arc<ScriptedProvider>| SessionConfig {
        workspace_root: root.to_path_buf(),
        provider: stub,
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: Some(4_000),
        compact_at: None,
        model_input: Vec::new(),
        model_params: None,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    };
    async fn drain(rx: &mut agent_kernel::channels::UiChannels) -> Vec<UiEvent> {
        rx.event_rx.close();
        let mut out = Vec::new();
        while let Some(ev) = rx.event_rx.recv().await {
            out.push(ev);
        }
        out
    }

    // Control: the default 80% leaves this fill alone — the only scripted
    // sample is the plain answer.
    let dir_a = tempfile::tempdir().unwrap();
    let stub_a = Arc::new(ScriptedProvider::new());
    stub_a.script_text("plain answer");
    let (mut a, mut ch_a) =
        SessionActor::resume(config(dir_a.path(), stub_a), SESSION_ID, seeded());
    a.handle(UiCommand::Prompt { text: "hi".into() }).await;
    let events_a = drain(&mut ch_a).await;
    assert!(
        events_a
            .iter()
            .all(|e| !matches!(e, UiEvent::Compacted { .. })),
        "the 80% default must not fire at ~72% fill"
    );

    // Retuned live: 70% → the very next turn compacts before its first sample.
    let dir_b = tempfile::tempdir().unwrap();
    let stub_b = Arc::new(ScriptedProvider::new());
    stub_b.script_text(SUMMARY); // the compactor's sample
    stub_b.script_text("answer after retune");
    let (mut b, mut ch_b) =
        SessionActor::resume(config(dir_b.path(), stub_b), SESSION_ID, seeded());
    b.handle(UiCommand::SetCompactAt { fraction: 0.7 }).await;
    b.handle(UiCommand::Prompt { text: "hi".into() }).await;

    let events_b = drain(&mut ch_b).await;
    assert!(
        events_b
            .iter()
            .any(|e| matches!(e, UiEvent::Compacted { .. })),
        "a live session must compact at the newly set 70%"
    );
    let final_text = events_b
        .iter()
        .rev()
        .find_map(|e| match e {
            UiEvent::AssistantMessage(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(final_text, "answer after retune");
    assert!(b
        .history()
        .iter()
        .any(|m| m.notice == Some(NoticeKind::CompactedMemory)));
}
