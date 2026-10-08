//! Blueprint acceptance: the system prompt carries the node index only when
//! `.blueprint/` exists, the `blueprint` tool is offered in plan mode, and a
//! turn whose writes break a blueprint rule buys exactly one hidden rework
//! round plus a persisted report.

use std::sync::Arc;

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
mod common;
use agent_llm::types::{NoticeKind, StreamChunk};
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

fn tool_call(name: &str, args: serde_json::Value) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
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

fn write(root: &std::path::Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn system_prompt(actor: &SessionActor) -> String {
    actor.history()[0].content.clone().unwrap_or_default()
}

fn hidden_texts(actor: &SessionActor) -> Vec<String> {
    actor
        .history()
        .iter()
        .filter(|m| m.notice == Some(NoticeKind::Hidden))
        .filter_map(|m| m.content.clone())
        .collect()
}

#[tokio::test]
async fn no_blueprint_dir_means_no_index_and_no_check() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "apply_patch",
        serde_json::json!({"patch": "*** Begin Patch\n*** Add File: loose/a.rs\n+fn a() {}\n*** End Patch\n"}),
    )));
    stub.script_text("done");
    let (mut actor, _ch) = spawn(dir.path(), stub.clone(), "build");

    assert!(system_prompt(&actor).contains("architecture layer off"));
    actor
        .handle(UiCommand::Prompt {
            text: "add a".into(),
        })
        .await;
    assert_eq!(stub.calls.lock().unwrap().len(), 2, "no rework round");
    assert!(hidden_texts(&actor).is_empty());
}

#[tokio::test]
async fn index_in_prompt_and_tool_in_plan_mode() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".blueprint/core.md",
        "+++\nowns = [\"src/**\"]\n+++\nCore engine.\n",
    );
    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("ok");
    let (mut actor, _ch) = spawn(dir.path(), stub.clone(), "plan");

    let prompt = system_prompt(&actor);
    assert!(
        prompt.contains("- core — Core engine. [src/**]"),
        "{prompt}"
    );
    actor
        .handle(UiCommand::Prompt {
            text: "where?".into(),
        })
        .await;
    let names = stub.tool_names.lock().unwrap()[0].clone();
    assert!(names.contains(&"blueprint".to_string()), "{names:?}");

    // A node added between turns reaches the next request's prompt.
    write(
        dir.path(),
        ".blueprint/ui.md",
        "+++\nowns = [\"web/**\"]\n+++\nThe web shell.\n",
    );
    stub.script_text("ok");
    actor
        .handle(UiCommand::Prompt {
            text: "again".into(),
        })
        .await;
    assert!(system_prompt(&actor).contains("- ui — The web shell."));
}

/// An unowned new file is a blueprint error: one hidden rework nudge, then
/// (still broken) the turn ends with the report event and a persisted row.
#[tokio::test]
async fn rule_violation_buys_one_rework_round_then_reports() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".blueprint/core.md",
        "+++\nowns = [\"src/**\"]\n+++\nCore engine.\n",
    );
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "apply_patch",
        serde_json::json!({"patch": "*** Begin Patch\n*** Add File: loose/a.rs\n+fn a() {}\n*** End Patch\n"}),
    )));
    stub.script_text("added it");
    // Rework round: the model answers without fixing.
    stub.script_text("left as is");
    let (mut actor, mut ch) = spawn(dir.path(), stub.clone(), "build");

    actor
        .handle(UiCommand::Prompt {
            text: "add a".into(),
        })
        .await;

    assert_eq!(
        stub.calls.lock().unwrap().len(),
        3,
        "exactly one rework round"
    );
    let hidden = hidden_texts(&actor);
    assert_eq!(hidden.len(), 1, "{hidden:?}");
    assert!(hidden[0].contains("loose/a.rs"), "{}", hidden[0]);

    ch.event_rx.close();
    let mut report = None;
    while let Some(ev) = ch.event_rx.recv().await {
        if let UiEvent::BlueprintReport { errors, text, .. } = ev {
            report = Some((errors, text));
        }
    }
    let (errors, text) = report.expect("BlueprintReport emitted");
    assert_eq!(errors, 1);
    assert!(text.contains("belongs to no node"), "{text}");
    assert!(
        actor
            .history()
            .iter()
            .any(|m| m.notice == Some(NoticeKind::Error)
                && m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("loose/a.rs"))),
        "report row persisted"
    );
}

/// A rework that fixes the violation (here: claiming the path in the node
/// file) ends the turn cleanly — no report.
#[tokio::test]
async fn fixed_violation_ends_clean() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".blueprint/core.md",
        "+++\nowns = [\"src/**\"]\n+++\nCore engine.\n",
    );
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "apply_patch",
        serde_json::json!({"patch": "*** Begin Patch\n*** Add File: loose/a.rs\n+fn a() {}\n*** End Patch\n"}),
    )));
    stub.script_text("added it");
    stub.push_script(tool_call(
        "apply_patch",
        serde_json::json!({"patch": "*** Begin Patch\n*** Update File: .blueprint/core.md\n@@\n-owns = [\"src/**\"]\n+owns = [\"src/**\", \"loose/**\"]\n*** End Patch\n"}),
    ));
    stub.script_text("claimed loose/ in core");
    let (mut actor, mut ch) = spawn(dir.path(), stub.clone(), "build");

    actor
        .handle(UiCommand::Prompt {
            text: "add a".into(),
        })
        .await;

    assert_eq!(stub.calls.lock().unwrap().len(), 4);
    ch.event_rx.close();
    while let Some(ev) = ch.event_rx.recv().await {
        assert!(
            !matches!(ev, UiEvent::BlueprintReport { .. }),
            "clean turn must not report: {ev:?}"
        );
    }
}

/// Goal mode: a `goal_complete` declared over a blueprint error is
/// withdrawn — the model is pushed back and has to declare again.
#[tokio::test]
async fn goal_completion_is_reopened_on_violation() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".blueprint/core.md",
        "+++\nowns = [\"src/**\"]\n+++\nCore engine.\n",
    );
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "apply_patch",
        serde_json::json!({"patch": "*** Begin Patch\n*** Add File: loose/a.rs\n+fn a() {}\n*** End Patch\n"}),
    )));
    stub.push_script(tool_call(
        "goal_complete",
        serde_json::json!({"summary": "added a"}),
    ));
    stub.script_text("done");
    // Rework round: no fix, no declaration → the goal contract pushes back…
    stub.script_text("hmm");
    // …and the model declares the blocker this time.
    stub.push_script(tool_call(
        "goal_blocked",
        serde_json::json!({"reason": "needs an owner decision"}),
    ));
    stub.script_text("blocked");
    let (mut actor, _ch) = spawn(dir.path(), stub.clone(), "goal");

    actor
        .handle(UiCommand::Prompt {
            text: "add a".into(),
        })
        .await;

    let hidden = hidden_texts(&actor);
    assert!(hidden[0].contains("完成宣告已撤回"), "{hidden:?}");
    assert!(
        hidden.iter().any(|h| h.contains("目标尚未宣告完成")),
        "the re-opened goal was pushed back: {hidden:?}"
    );
    assert_eq!(stub.calls.lock().unwrap().len(), 6);
}

/// `bash` writes never reach the hunk tracker; the turn-start file snapshot
/// still catches a file it created outside every node.
#[tokio::test]
async fn bash_created_file_outside_every_node_is_caught() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".blueprint/core.md",
        "+++\nowns = [\"src/**\"]\n+++\nCore engine.\n",
    );
    let stub = Arc::new(ScriptedProvider::new().with_script(tool_call(
        "bash",
        serde_json::json!({"command": "mkdir -p loose && echo x > loose/b.rs"}),
    )));
    stub.script_text("made it");
    stub.script_text("left as is");
    let (mut actor, _ch) = spawn(dir.path(), stub.clone(), "build");

    actor
        .handle(UiCommand::Prompt {
            text: "make b".into(),
        })
        .await;

    let hidden = hidden_texts(&actor);
    assert_eq!(hidden.len(), 1, "{hidden:?}");
    assert!(hidden[0].contains("loose/b.rs"), "{}", hidden[0]);
}
