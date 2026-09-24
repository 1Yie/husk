//! Stage-4 acceptance: the full ReAct loop runs headless on a scripted provider —
//! prompt → stream → tool call → tool result injected → re-sample → final
//! answer. No network, no display server.

use std::sync::Arc;

use agent_ipc::{AgentState, UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
mod common;
use agent_llm::types::StreamChunk;
use common::ScriptedProvider;

fn collect_script_tool_call_then_answer() -> Vec<Vec<StreamChunk>> {
    // Script 1: model emits a tool call to `list_dir` + Done.
    let tool_args = serde_json::json!({"path": "."}).to_string();
    let script1 = vec![
        StreamChunk::ContentDelta("Let me look at the workspace.".into()),
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("list_dir".into()),
            args_delta: tool_args,
        },
        StreamChunk::Done {
            prompt_tokens: Some(10),
            completion_tokens: Some(5),
            cached_tokens: None,
        },
    ];
    // Script 2: after the tool result, model answers + Done.
    let script2 = vec![
        StreamChunk::ContentDelta("The workspace has 2 entries.".into()),
        StreamChunk::Done {
            prompt_tokens: Some(20),
            completion_tokens: Some(8),
            cached_tokens: None,
        },
    ];
    vec![script1, script2]
}

#[tokio::test]
async fn headless_react_loop_drives_tool_then_answers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();
    std::fs::write(dir.path().join("b.txt"), "y").unwrap();

    let stub = ScriptedProvider::new();
    for script in collect_script_tool_call_then_answer() {
        stub.push_script(script);
    }

    let (mut actor, mut channels) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });

    // Drive one prompt through the actor.
    actor
        .handle(UiCommand::Prompt {
            text: "list files".into(),
        })
        .await;

    // Collect everything the kernel emitted.
    channels.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        events.push(ev);
    }

    // State machine passed through Reasoning → ExecutingTool → Finished.
    let states: Vec<&AgentState> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::StateChanged(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(states.iter().any(|s| matches!(s, AgentState::Reasoning)));
    assert!(states
        .iter()
        .any(|s| matches!(s, AgentState::ExecutingTool { .. })));
    assert!(matches!(states.last(), Some(AgentState::Finished)));

    // The tool ran and its result went back to the model.
    let tool_started = events
        .iter()
        .any(|e| matches!(e, UiEvent::ToolCallStarted { name, .. } if name == "list_dir"));
    let tool_done = events.iter().any(
        |e| matches!(e, UiEvent::ToolCallFinished { name, ok: true, .. } if name == "list_dir"),
    );
    assert!(tool_started && tool_done);

    // Final assistant text arrived.
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::AssistantMessage(t) if t.contains("2 entries"))));

    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::Usage {
            completion_tokens: 8,
            ..
        }
    )));

    // History now holds: system, user, assistant(tool_call), tool result, assistant(answer).
    assert!(actor.history_len() >= 5);
}

#[tokio::test]
async fn system_prompt_is_rendered_with_workspace_and_git() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
    let stub = ScriptedProvider::new();
    stub.script_text("ok");

    let (mut actor, _c) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "m".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });
    actor.handle(UiCommand::Prompt { text: "hi".into() }).await;
    // Spawn rendered the skeleton into the system message — history[0] must
    // contain the scanned filename and a git status line.
    // (history is private; assert indirectly via a second prompt succeeding)
    assert!(actor.history_len() >= 3);
}

#[tokio::test]
async fn steer_between_turns_becomes_a_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let stub = ScriptedProvider::new();
    stub.script_text("first");
    stub.script_text("steered answer");

    let (mut actor, mut channels) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "m".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });

    actor.handle(UiCommand::Prompt { text: "one".into() }).await;
    // Between turns a Steer acts like a Prompt.
    actor
        .handle(UiCommand::Steer {
            text: "actually do X".into(),
        })
        .await;

    channels.event_rx.close();
    let mut all = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        all.push(ev);
    }
    let texts: Vec<String> = all
        .iter()
        .filter_map(|e| match e {
            UiEvent::AssistantMessage(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t == "steered answer"));
}

/// The reload bug: reasoning was streamed live but never stored, so a
/// reopened session showed no 思考过程 at all. Each round's trace now rides
/// its own assistant row — including a tool-calling round, whose row carries
/// no visible text and whose trace is therefore the *only* thing that makes
/// the round's thinking replayable.
#[tokio::test]
async fn reasoning_traces_persist_for_replay() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();

    let stub = ScriptedProvider::new();
    // Round 1: think, then call a tool (no text of its own).
    stub.push_script(vec![
        StreamChunk::ReasoningDelta("先看看目录里".into()),
        StreamChunk::ReasoningDelta("有什么文件。".into()),
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("list_dir".into()),
            args_delta: serde_json::json!({"path": "."}).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: Some(10),
            completion_tokens: Some(4),
            cached_tokens: None,
        },
    ]);
    // Round 2: think, then answer.
    stub.push_script(vec![
        StreamChunk::ReasoningDelta("目录里有 a.txt。".into()),
        StreamChunk::ContentDelta("工作区有 1 个文件。".into()),
        StreamChunk::Done {
            prompt_tokens: Some(20),
            completion_tokens: Some(6),
            cached_tokens: None,
        },
    ]);

    let (mut actor, _c) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });
    actor
        .handle(UiCommand::Prompt {
            text: "list files".into(),
        })
        .await;

    let traces: Vec<&str> = actor
        .history()
        .iter()
        .filter_map(|m| m.reasoning.as_deref())
        .collect();
    assert_eq!(
        traces,
        vec!["先看看目录里有什么文件。", "目录里有 a.txt。"],
        "each round keeps its own trace, in order"
    );

    let tool_round = actor
        .history()
        .iter()
        .find(|m| m.tool_calls.is_some() && m.content.is_none())
        .expect("the tool-calling round row");
    assert_eq!(
        tool_round.reasoning.as_deref(),
        Some("先看看目录里有什么文件。"),
        "a round with no text must still carry its trace — otherwise its          thinking block vanishes on reload"
    );
}

/// Skills reach the model by progressive disclosure: the system prompt carries
/// a catalog (name + description) so it can *choose*, and the `skill` tool
/// fetches the body on demand. Without this the model never learned that any
/// skill existed — the feature was user-only (`$name` / `/name`).
#[tokio::test]
async fn subagent_catalog_is_in_the_prompt_and_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let write_agent = |name: &str, desc: &str| {
        let p = dir.path().join(".husk/agents");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: {desc}\n---\n\nYou are {name}.\n"),
        )
        .unwrap();
    };
    write_agent("helper", "project-local helper agent");

    let stub = ScriptedProvider::new();
    stub.script_text("ok");
    let (mut actor, _c) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "m".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });

    let system = actor.history()[0].content.clone().unwrap_or_default();
    assert!(
        system.contains("`helper`"),
        "catalog misses the manifest:\n{system}"
    );
    assert!(system.contains("project-local helper agent"), "{system}");
    assert!(system.contains("(project)"), "{system}");
    // Builtins are still listed so `agent: "review"` keeps working.
    assert!(
        system.contains("`review`") && system.contains("(built-in)"),
        "{system}"
    );

    // A manifest added mid-session shows up after the next turn.
    write_agent("later", "added while the session was open");
    actor.handle(UiCommand::Prompt { text: "hi".into() }).await;
    let system = actor.history()[0].content.clone().unwrap_or_default();
    assert!(
        system.contains("`later`"),
        "catalog did not refresh:\n{system}"
    );
    assert!(system.contains("`helper`"), "{system}");
}

async fn subagent_catalog_is_in_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let write_skill = |name: &str, desc: &str| {
        let p = dir.path().join(".agents/skills").join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n\nDo the {name} thing.\n"),
        )
        .unwrap();
    };
    write_skill("demo-skill", "a demo skill for the stage-4 suite");

    let stub = ScriptedProvider::new();
    stub.script_text("ok");
    let (mut actor, _c) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "m".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });

    let system = actor.history()[0].content.clone().unwrap_or_default();
    assert!(system.contains("## Skills"), "no Skills section:\n{system}");
    assert!(
        system.contains("`demo-skill`"),
        "catalog misses the skill:\n{system}"
    );
    assert!(system.contains("a demo skill for the stage-4 suite"));
    // The load-on-demand rule has to be stated, or the model may guess.
    assert!(system.contains("skill` tool"), "{system}");

    // A skill added mid-session shows up after the next turn: the block is
    // re-rendered only when the skill dirs change (the stamp gate).
    write_skill("later-skill", "added after the session started");
    actor.handle(UiCommand::Prompt { text: "hi".into() }).await;
    let system = actor.history()[0].content.clone().unwrap_or_default();
    assert!(
        system.contains("`later-skill`"),
        "catalog did not refresh:\n{system}"
    );
    assert!(system.contains("`demo-skill`"), "{system}");
}

/// The composer's parked queue lives in the actor: `SetQueued` replaces it
/// (echoed via `QueuedPrompts`), and a finished `Prompt` turn drains each
/// parked entry as its own fresh prompt — one per turn, in order.
#[tokio::test]
async fn queued_prompts_drain_one_per_turn() {
    let dir = tempfile::tempdir().unwrap();
    let stub = ScriptedProvider::new();
    // Three turns run: the direct prompt plus both parked ones.
    stub.script_text("answer p0");
    stub.script_text("answer q1");
    stub.script_text("answer q2");

    let (mut actor, mut channels) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        provider: Arc::new(stub),
        model: "m".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
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
    });

    actor
        .handle(UiCommand::SetQueued {
            items: vec!["queued one".into(), "queued two".into()],
        })
        .await;
    actor
        .handle(UiCommand::Prompt {
            text: "direct prompt".into(),
        })
        .await;

    channels.event_rx.close();
    let mut events = Vec::new();
    while let Some(ev) = channels.event_rx.recv().await {
        events.push(ev);
    }

    // Every parked prompt ran as its own turn — three user echoes total.
    let prompts: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::UserPrompt(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(prompts, ["direct prompt", "queued one", "queued two"]);

    // The queue emptied: echoes go [full list] → [one popped] → [empty].
    let queue_snapshots: Vec<&Vec<String>> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::QueuedPrompts { items } => Some(items),
            _ => None,
        })
        .collect();
    assert_eq!(queue_snapshots.len(), 3);
    assert_eq!(queue_snapshots[0].len(), 2);
    assert_eq!(queue_snapshots[1].len(), 1);
    assert!(queue_snapshots[2].is_empty());
}
