//! Lifecycle-hook acceptance: a plugin's declared `hooks[]` reach the state
//! machine and change what happens — a veto, an argument rewrite, a blocked
//! turn, and a state-transition feed — driven end to end through the real
//! actor + engine on a scripted provider.
//!
//! The plugin side is exercised the way discovery exercises it: a real
//! `manifest.json` in a plugin dir, parsed and converted into chain entries;
//! only the MCP handshake is skipped (a hook needs no server).

use std::sync::Arc;

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::hooks::{chain_from_specs, HookChain};
use agent_kernel::session::{SessionActor, SessionConfig};
mod common;
use agent_llm::types::StreamChunk;
use common::ScriptedProvider;

/// Write `.husk/plugins/<id>/manifest.json` + an executable hook script, and
/// return the chain the kernel would build from it.
fn plugin_chain(ws: &std::path::Path, id: &str, hooks: serde_json::Value) -> HookChain {
    let dir = ws.join(".husk/plugins").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            // A pure plugin — hooks need no `kind`/`entry`; nothing connects.
            "id": id, "name": id,
            "capabilities": {"hooks": hooks},
        }))
        .unwrap(),
    )
    .unwrap();

    // Parse it the way `load_all` does (including the discovery-fed `dir`).
    let text = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    let mut manifest: agent_plugin::PluginManifest = serde_json::from_str(&text).unwrap();
    manifest.dir = dir;
    manifest.validate().expect("manifest validates");
    chain_from_specs(agent_plugin::hooks_from_manifest(&manifest))
}

fn script(ws: &std::path::Path, id: &str, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let p = ws.join(".husk/plugins").join(id).join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut perm = std::fs::metadata(&p).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&p, perm).unwrap();
}

fn actor_for(
    ws: &std::path::Path,
    stub: Arc<ScriptedProvider>,
) -> (SessionActor, agent_kernel::channels::UiChannels) {
    SessionActor::spawn(SessionConfig {
        workspace_root: ws.to_path_buf(),
        provider: stub,
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
    })
}

fn drain(rx: &mut agent_kernel::channels::UiReceiver) -> Vec<UiEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.try_recv() {
        events.push(ev);
    }
    events
}

fn tool_call_script(tool: &str, args: serde_json::Value) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some(tool.into()),
            args_delta: args.to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: Some(10),
            completion_tokens: Some(5),
            cached_tokens: None,
        },
    ]
}

fn answer_script(text: &str) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ContentDelta(text.into()),
        StreamChunk::Done {
            prompt_tokens: Some(20),
            completion_tokens: Some(8),
            cached_tokens: None,
        },
    ]
}

/// A vetoing plugin stops the tool before it runs, and its reason is what the
/// model is told — not a generic "denied".
#[tokio::test]
async fn before_tool_hook_vetoes_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let chain = plugin_chain(
        ws,
        "guard",
        serde_json::json!([{
            "event": "before_tool_execute",
            "filter": {"tool": "list_dir"},
            "run": {"command": "./veto.sh"},
        }]),
    );
    script(ws, "guard", "veto.sh", r#"echo '{"action":"veto","reason":"blocked by guard policy"}'"#);

    let stub = ScriptedProvider::new();
    stub.push_script(tool_call_script("list_dir", serde_json::json!({"path": "."})));
    stub.push_script(answer_script("understood"));
    let (mut actor, mut channels) = actor_for(ws, Arc::new(stub));
    actor.set_hooks(chain);

    actor
        .handle(UiCommand::Prompt {
            text: "list files".into(),
        })
        .await;

    let events = drain(&mut channels.event_rx);
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::ToolCallFinished { name, ok: false, content, .. }
                if name == "list_dir" && content.contains("blocked by guard policy")
        )),
        "veto reason did not reach the tool result: {events:?}"
    );
    // The model saw the refusal injected as the tool's result.
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::AssistantMessage(t) if t.contains("understood"))));
}

/// A rewriting plugin changes what the tool was actually called with.
#[tokio::test]
async fn before_tool_hook_rewrites_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("rewritten")).unwrap();
    std::fs::write(ws.join("rewritten/marker.txt"), "x").unwrap();

    let chain = plugin_chain(
        ws,
        "rewriter",
        serde_json::json!([{
            "event": "before_tool_execute",
            "filter": {"tool": "list_dir"},
            "run": {"command": "./rewrite.sh"},
        }]),
    );
    script(
        ws,
        "rewriter",
        "rewrite.sh",
        r#"echo '{"action":"rewrite","args":{"path":"rewritten"}}'"#,
    );

    let stub = ScriptedProvider::new();
    // The model asks for the workspace root; the hook redirects it.
    stub.push_script(tool_call_script("list_dir", serde_json::json!({"path": "."})));
    stub.push_script(answer_script("done"));
    let (mut actor, mut channels) = actor_for(ws, Arc::new(stub));
    actor.set_hooks(chain);

    actor
        .handle(UiCommand::Prompt {
            text: "list".into(),
        })
        .await;

    let events = drain(&mut channels.event_rx);
    // `marker.txt` only exists under `rewritten/`, so seeing it proves the
    // tool ran with the hook's arguments rather than the model's.
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::ToolCallFinished { name, ok: true, content, .. }
                if name == "list_dir" && content.contains("marker.txt")
        )),
        "rewritte args never reached the tool: {events:?}"
    );
}

/// `on_user_input` can refuse the turn outright — no sampling happens.
#[tokio::test]
async fn user_input_hook_blocks_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let chain = plugin_chain(
        ws,
        "gate",
        serde_json::json!([{
            "event": "on_user_input",
            "run": {"command": "./block.sh"},
        }]),
    );
    script(ws, "gate", "block.sh", r#"echo '{"action":"block","reason":"input rejected by gate"}'"#);

    let stub = ScriptedProvider::new();
    stub.push_script(answer_script("should never be sent"));
    let (mut actor, mut channels) = actor_for(ws, Arc::new(stub));
    actor.set_hooks(chain);

    actor
        .handle(UiCommand::Prompt {
            text: "do something forbidden".into(),
        })
        .await;

    let events = drain(&mut channels.event_rx);
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::SystemMessage(m) if m.contains("input rejected by gate")
        )),
        "block reason missing from the transcript: {events:?}"
    );
    // No assistant output: the turn never reached the provider.
    assert!(!events
        .iter()
        .any(|e| matches!(e, UiEvent::AssistantMessage(t) if t.contains("should never be sent"))));
}

/// Every state the kernel enters is reported to an `on_state_transition`
/// hook, and a repeated state is not reported twice.
#[tokio::test]
async fn state_transition_hook_sees_each_change_once() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let chain = plugin_chain(
        ws,
        "audit",
        serde_json::json!([{
            "event": "on_state_transition",
            "run": {"command": "./log.sh"},
        }]),
    );
    // The payload on stdin IS the log line.
    script(ws, "audit", "log.sh", "cat >> transitions.log");

    let stub = ScriptedProvider::new();
    stub.push_script(tool_call_script("list_dir", serde_json::json!({"path": "."})));
    stub.push_script(answer_script("done"));
    let (mut actor, mut channels) = actor_for(ws, Arc::new(stub));
    actor.set_hooks(chain);

    actor
        .handle(UiCommand::Prompt {
            text: "list".into(),
        })
        .await;
    // Transition hooks are dispatched off the turn's hot path — let them land.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let log = std::fs::read_to_string(ws.join(".husk/plugins/audit/transitions.log"))
        .expect("transition hook never ran");
    // Each payload is one write, so each `cat` appends one line — but two
    // transition hooks CAN run concurrently and a torn line is the hook
    // author's problem, not the kernel's, so skip what does not parse.
    let states: Vec<String> = log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v["new"].as_str().map(String::from))
        .collect();
    assert!(!states.is_empty(), "no parsable payload in {log:?}");
    for expected in ["ScanningWorkspace", "Reasoning", "ExecutingTool", "Finished"] {
        assert!(states.contains(&expected.to_string()), "{expected} missing: {states:?}");
    }
    // No state repeats back-to-back (the shared slot de-duplicates the
    // engine's announcement and the session's own mirror of it).
    for pair in states.windows(2) {
        assert_ne!(pair[0], pair[1], "duplicate transition in {states:?}");
    }
    let _ = drain(&mut channels.event_rx);
}

/// An `on_response` hook rewrites the assistant's final answer — what lands
/// in history and on screen carries the hook's edit.
#[tokio::test]
async fn response_hook_rewrites_the_final_answer() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let chain = plugin_chain(
        ws,
        "meow",
        serde_json::json!([{
            "event": "on_response",
            "run": {"command": "./meow.sh"},
        }]),
    );
    // stdin carries {"event","text"}; the verdict rewrites `text`.
    script(
        ws,
        "meow",
        "meow.sh",
        r#"python3 -c 'import json,sys; p=json.load(sys.stdin); print(json.dumps({"action":"rewrite","text":p["text"]+" 喵～"}))'"#,
    );

    let stub = ScriptedProvider::new();
    stub.push_script(answer_script("plain answer"));
    let (mut actor, mut channels) = actor_for(ws, Arc::new(stub));
    actor.set_hooks(chain);

    actor
        .handle(UiCommand::Prompt {
            text: "say hi".into(),
        })
        .await;

    let events = drain(&mut channels.event_rx);
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::AssistantMessage(t) if t.contains("plain answer 喵～")
        )),
        "on_response rewrite missing from the answer: {events:?}"
    );
}

/// A hook only fires on its declared event — a `before_tool_execute` hook
/// must not be spawned for user input, a tool-less answer, or any state
/// transition (it used to: the dispatch had no event guard).
#[tokio::test]
async fn hooks_fire_only_on_their_declared_event() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let chain = plugin_chain(
        ws,
        "spy",
        serde_json::json!([{
            "event": "before_tool_execute",
            "run": {"command": "./spy.sh"},
        }]),
    );
    // Any spawn leaves a marker — the file's existence is the assertion.
    script(ws, "spy", "spy.sh", "echo ran >> spawns.log");

    let stub = ScriptedProvider::new();
    // A plain answer turn: user input, no tool calls, several transitions.
    stub.push_script(answer_script("no tools today"));
    let (mut actor, mut channels) = actor_for(ws, Arc::new(stub));
    actor.set_hooks(chain);

    actor
        .handle(UiCommand::Prompt {
            text: "just talk".into(),
        })
        .await;
    // Transition hooks dispatch off the turn's hot path — give them a beat,
    // then check the before_tool hook was never spawned.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert!(
        !ws.join(".husk/plugins/spy/spawns.log").exists(),
        "before_tool_execute hook was spawned on a non-tool event"
    );
    let _ = drain(&mut channels.event_rx);
}
