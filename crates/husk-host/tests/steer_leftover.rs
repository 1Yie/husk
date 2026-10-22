//! A steer that outlived its turn must still be delivered.
//!
//! The engine drains its steer channel at the top of every sample round, so a
//! steer landing while the LAST round is already sampling has no round left to
//! be injected into. It used to sit in the channel until the next prompt's
//! turn — the composer's 立即发送 looked like it did nothing. The actor now
//! runs such a steer as the next prompt itself.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_ipc::UiEvent;
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

fn config(dir: &std::path::Path, provider: Arc<ScriptedProvider>) -> SessionConfig {
    SessionConfig {
        workspace_root: dir.to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider,
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
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    }
}

/// Wait until the provider has been entered. The gate is `take()`n by the
/// request that reaches it, so an empty slot is the deterministic "the round
/// is sampling" signal — and sampling happens AFTER the round's steer drain.
async fn wait_for_the_provider(stub: &ScriptedProvider) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while stub.gate.lock().unwrap().is_some() {
        assert!(
            Instant::now() < deadline,
            "the turn never reached the provider"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[test]
fn a_steer_landing_in_the_last_round_runs_as_the_next_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("first answer");
    stub.script_text("steered answer");
    stub.script_text("parked answer");
    // The gate parks the turn's only round inside the provider — past the
    // round's steer drain. The script calls no tool, so the turn ends with
    // the steer still undelivered.
    let gate = Arc::new(tokio::sync::Notify::new());
    *stub.gate.lock().unwrap() = Some(gate.clone());

    let rt = tokio::runtime::Runtime::new().unwrap();
    // One parked follow-up too, so the delivery ORDER is under test as well:
    // the late steer was typed before that queue drained and must go first.
    let mut cfg = config(dir.path(), stub.clone());
    cfg.queued_prompts = vec!["parked".into()];
    let (mut actor, mut channels) = rt.block_on(async { SessionActor::spawn(cfg) });
    let steer_tx = actor.steer_writer();
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt { text: "one".into() }))
        .unwrap();

    let (prompts, answers, steers) = rt.block_on(async {
        wait_for_the_provider(&stub).await;
        // The IPC layer's Steer path: straight into the engine's channel,
        // never through the command pump.
        steer_tx.try_send("actually do X".into()).unwrap();
        gate.notify_one();

        let mut prompts = Vec::new();
        let mut answers = Vec::new();
        let mut steers = 0usize;
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(ev) = channels.event_rx.recv().await {
                match ev {
                    UiEvent::UserPrompt(text) => prompts.push(text),
                    UiEvent::AssistantMessage(text) => answers.push(text),
                    UiEvent::UserSteer(_) => steers += 1,
                    _ => {}
                }
                // Three turns: the prompt, the late steer, the parked one.
                if prompts.len() == 3 && answers.len() == 3 {
                    break;
                }
            }
        })
        .await
        .expect("the late steer and the parked prompt must both run");
        (prompts, answers, steers)
    });

    assert_eq!(steers, 0, "the steer was injected into a finished round");
    // The late steer opens a turn of its own, exactly like a typed prompt —
    // and outranks the prompt parked before it.
    assert_eq!(
        prompts,
        vec![
            "one".to_string(),
            "actually do X".to_string(),
            "parked".to_string()
        ]
    );
    assert!(answers.iter().any(|a| a.contains("first answer")));
    assert!(answers.iter().any(|a| a.contains("steered answer")));
    assert!(answers.iter().any(|a| a.contains("parked answer")));
    assert_eq!(
        stub.calls.lock().unwrap().len(),
        3,
        "one request per turn: {prompts:?} {answers:?}"
    );
}

#[test]
fn a_steer_arriving_while_the_session_sits_idle_still_runs() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("first answer");
    stub.script_text("steered answer");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, mut channels) =
        rt.block_on(async { SessionActor::spawn(config(dir.path(), stub.clone())) });
    let steer_tx = actor.steer_writer();
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt { text: "one".into() }))
        .unwrap();

    let answers = rt.block_on(async {
        // Wait for the turn to settle, then steer: the pump is idle, so the
        // channel has no reader other than the actor itself.
        let mut answers = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(ev) = channels.event_rx.recv().await {
                if let UiEvent::AssistantMessage(text) = ev {
                    answers.push(text);
                    break;
                }
            }
        })
        .await
        .expect("the first turn answered");

        steer_tx.try_send("actually do X".into()).unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(ev) = channels.event_rx.recv().await {
                if let UiEvent::AssistantMessage(text) = ev {
                    answers.push(text);
                    break;
                }
            }
        })
        .await
        .expect("an idle steer must open its own turn");
        answers
    });

    assert!(answers[0].contains("first answer"));
    assert!(answers[1].contains("steered answer"));
    assert_eq!(stub.calls.lock().unwrap().len(), 2, "one request per turn");
}
