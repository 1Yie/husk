//! Plan-mode acceptance: the tool schema the model is offered, the
//! `submit_plan` deliverable (event + persisted replay row), and the
//! single follow-up nudge. The registry composition itself was only covered
//! incidentally (stage3 asserts the builtin set), and nothing exercised the
//! submit path end to end.

use std::sync::Arc;

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
mod common;
use agent_llm::types::{NoticeKind, StreamChunk};
use common::ScriptedProvider;

fn spawn_plan(
    dir: &std::path::Path,
    stub: Arc<ScriptedProvider>,
) -> (
    SessionActor,
    agent_kernel::channels::UiChannels,
) {
    SessionActor::spawn(SessionConfig {
        workspace_root: dir.to_path_buf(),
        provider: stub,
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "plan".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        model_input: Vec::new(),
        model_params: None,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    })
}

fn done() -> StreamChunk {
    StreamChunk::Done {
        prompt_tokens: Some(5),
        completion_tokens: Some(5),
        cached_tokens: None,
    }
}

fn tool_call(name: &str, args: serde_json::Value) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some(name.into()),
            args_delta: args.to_string(),
        },
        done(),
    ]
}

/// The model must see `submit_plan` and must NOT see write tools — a write
/// tool in the schema is a write tool the model can be talked into calling.
#[tokio::test]
async fn plan_schema_offers_submit_plan_and_no_writers() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("只读分析，无需交付方案。");
    let (mut actor, _ch) = spawn_plan(dir.path(), stub.clone());

    actor
        .handle(UiCommand::Prompt {
            text: "registry 是怎么组织的？".into(),
        })
        .await;

    let names = stub
        .tool_names
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("a request went out");
    assert!(names.contains(&"submit_plan".to_string()), "{names:?}");
    for readonly in ["smart_read", "smart_grep", "list_dir"] {
        assert!(names.contains(&readonly.to_string()), "missing {readonly}: {names:?}");
    }
    for writer in ["fuzzy_patch", "apply_patch", "bash", "smart_test_runner"] {
        assert!(
            !names.contains(&writer.to_string()),
            "{writer} leaked into the plan-mode schema: {names:?}"
        );
    }

    // A question with no tool work is answering, not planning: no nudge.
    assert_eq!(stub.calls.lock().unwrap().len(), 1);
}

/// A submitted plan reaches the UI as its own event AND is persisted as a
/// tagged row, so a reopened session replays the card instead of re-parsing
/// prose. Once submitted, the turn ends without the nudge round.
#[tokio::test]
async fn submit_plan_emits_the_event_and_persists_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "submit_plan",
        serde_json::json!({
            "summary": "把 memory 存储拆成独立模块",
            "steps": [
                {"title": "新增 mod", "detail": "搬走存储实现", "files": ["store.rs"]}
            ],
            "verification": ["cargo test -p agent-context"],
            "risks": ["旧库需要迁移路径"]
        }),
    )));
    stub.script_text("方案已提交，请审阅。");
    let (mut actor, mut ch) = spawn_plan(dir.path(), stub.clone());

    actor
        .handle(UiCommand::Prompt {
            text: "规划存储拆分".into(),
        })
        .await;

    ch.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = ch.event_rx.recv().await {
        events.push(ev);
    }

    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::PlanSubmitted { plan } if plan.contains("把 memory 存储拆成独立模块"))),
        "no PlanSubmitted event"
    );
    assert!(
        actor
            .history()
            .iter()
            .any(|m| m.notice == Some(NoticeKind::Plan)),
        "the plan row was not persisted for replay"
    );
    // Submitted → the deliverable landed, no follow-up nudge.
    assert_eq!(stub.calls.lock().unwrap().len(), 2);
}

/// Doing tool work without submitting buys exactly ONE nudge
/// (`MAX_PLAN_FOLLOWUPS = 1`) — a plain question must not pay it twice.
#[tokio::test]
async fn an_unsubmitted_plan_gets_at_most_one_nudge() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();
    let stub = Arc::new(ScriptedProvider::new().with_script(vec![
        StreamChunk::ContentDelta("让我看看工作区。".into()),
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("list_dir".into()),
            args_delta: serde_json::json!({"path": "."}).to_string(),
        },
        done(),
    ]));
    // Round 2 answers in prose without submitting → this is what buys the
    // single nudge.
    stub.script_text("工作已完成。");
    // Round 3 is the nudge round; prose again must end the turn (the script
    // queue is exhausted after this, so a second nudge would fail loudly).
    stub.script_text("没有方案要交付。");
    let (mut actor, mut ch) = spawn_plan(dir.path(), stub.clone());

    actor
        .handle(UiCommand::Prompt {
            text: "规划一下".into(),
        })
        .await;

    ch.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = ch.event_rx.recv().await {
        events.push(ev);
    }

    assert_eq!(
        stub.calls.lock().unwrap().len(),
        3,
        "tool round + answer + exactly one nudge round"
    );
    assert!(
        matches!(
            events
                .iter()
                .rev()
                .find_map(|e| match e {
                    UiEvent::StateChanged(s) => Some(s),
                    _ => None,
                }),
            Some(agent_ipc::AgentState::Finished)
        ),
        "a nudged turn still finishes cleanly"
    );
    let nudges = actor
        .history()
        .iter()
        .filter(|m| {
            m.notice == Some(NoticeKind::Hidden)
                && m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("submit_plan"))
        })
        .count();
    assert_eq!(nudges, 1, "the nudge must not stack");
    assert!(
        !actor
            .history()
            .iter()
            .any(|m| m.notice == Some(NoticeKind::Plan)),
        "nothing submitted → no plan row"
    );
}
